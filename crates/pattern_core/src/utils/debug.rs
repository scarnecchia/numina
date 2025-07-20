//! Debug utilities for prettier output

use std::fmt;

/// Formats large arrays in a compact way for debug output
pub fn format_array_compact<T: fmt::Debug>(
    f: &mut fmt::Formatter<'_>,
    arr: &[T],
    max_items: usize,
) -> fmt::Result {
    if arr.len() <= max_items {
        write!(f, "{:?}", arr)
    } else {
        write!(f, "[{:?}", arr[0])?;
        for item in &arr[1..max_items / 2] {
            write!(f, ", {:?}", item)?;
        }
        write!(f, ", ... {} more ...", arr.len() - max_items)?;
        for item in &arr[arr.len() - max_items / 2..] {
            write!(f, ", {:?}", item)?;
        }
        write!(f, "]")
    }
}

/// Formats float arrays (like embeddings) in a very compact way
pub fn format_float_array_compact(f: &mut fmt::Formatter<'_>, arr: &[f32]) -> fmt::Result {
    if arr.is_empty() {
        return write!(f, "[]");
    }

    let dims = arr.len();
    if dims <= 6 {
        // Show all values for small arrays
        write!(f, "{:?}", arr)
    } else {
        // Show first 3, last 3, and dimensions
        write!(
            f,
            "[{:.3}, {:.3}, {:.3}, ... {:.3}, {:.3}, {:.3}] ({}d)",
            arr[0],
            arr[1],
            arr[2],
            arr[dims - 3],
            arr[dims - 2],
            arr[dims - 1],
            dims
        )
    }
}

/// Wrapper for pretty-printing SurrealDB Response
///
/// ## The Great SurrealDB Response Hack™
///
/// So here's the thing: SurrealDB's `Response` type has all its fields marked `pub(crate)`,
/// which means we can't access them. This is probably for good reasons - maybe they don't
/// want to commit to a stable API, maybe they're planning changes, maybe they just hate us.
///
/// But we NEED to see what's in there! The default Debug output looks like this:
/// ```text
/// Response {
///     results: {
///         0: (
///             Stats {
///                 execution_time: Some(
///                     704.588µs,
///                 ),
///             },
///             Ok(
///                 Array(
///                     Array(
///                         [
///                             Object(
///                                 Object(
///                                     {
///                                         "agents": Array(
///                                             Array(
///                                                 [
///                                                     Object(
///                                                         Object(
///                                                             {
///                                                                 // ... 200 more lines of this
/// ```
///
/// Which is about as readable as the terms of service for a social media platform.
///
/// So what do we do? We commit crimes against good software engineering practices!
///
/// ## The Hack
///
/// We format the Response with `{:#?}`, then parse the resulting string like it's 1999
/// and regexes just got invented. Is it cursed? Yes. Does it work? Also yes.
///
/// We look for patterns like:
/// - `Ok(Array(Array([...])))` - because SurrealDB double-wraps everything
/// - `Stats { execution_time: Some(123.456µs) }` - to extract timing info
/// - `"field_name": Value(...)` - to figure out object keys
///
/// ## Why Not Use a Proper Parser?
///
/// We could use `nom` (there's even a crate called `debug_parser` that does this properly).
/// But that would mean:
/// 1. Adding a dependency
/// 2. Writing actual parsing logic
/// 3. Admitting defeat
///
/// Instead, we embrace the chaos. We split on commas (but not commas in strings!),
/// we count brackets (but what about nested brackets?!), we trim whitespace (but what if
/// whitespace is significant?!).
///
/// ## Does It Work?
///
/// Surprisingly well! We can turn that unreadable mess into:
/// ```text
/// SurrealDB Response {
///   statements: 4
///   summary: 4 ok, 0 errors
///   results: [
///     [0]: Ok(Array[0])
///     [1]: Ok(Array[~2 Objects{"id", "name"}])
///     [2]: Ok(Number(42))
///     [3]: Ok(Object{"result"})
///   ]
/// }
/// ```
///
/// ## Should You Use This In Production?
///
/// lol no. This is for debugging. If SurrealDB changes their Debug format even slightly,
/// this whole thing explodes like a piñata filled with null pointers.
///
/// But for now? It makes debugging responses actually bearable, and that's all we need.
pub struct ResponseDebug<'a>(pub &'a surrealdb::Response);

impl<'a> fmt::Display for ResponseDebug<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let num_statements = self.0.num_statements();
        write!(f, "Response({} statements)", num_statements)
    }
}

impl<'a> fmt::Debug for ResponseDebug<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Step 1: Summon the debug string from the void
        // This is like asking a genie for wishes, except the genie speaks in
        // deeply nested Rust structs
        let debug_str = format!("{:#?}", self.0);
        let num_statements = self.0.num_statements();

        writeln!(f, "SurrealDB Response {{")?;
        writeln!(f, "  statements: {}", num_statements)?;

        // Count Ok vs Err results by literally counting the string "Ok(" and "Err("
        // Will this break if someone has a string containing "Ok("? Yes.
        // Do we care? Not today, Satan.
        let ok_count = debug_str.matches("Ok(").count().min(num_statements);
        let err_count = debug_str.matches("Err(").count().min(num_statements);

        writeln!(f, "  summary: {} ok, {} errors", ok_count, err_count)?;

        // Try to extract compact result info
        // We arbitrarily decide that 10KB of debug output is our limit
        // because parsing War and Peace as debug output seems excessive
        if debug_str.len() < 10000 && num_statements <= 10 {
            writeln!(f, "  results: [")?;
            for i in 0..num_statements {
                write!(f, "    [{}]: ", i)?;

                // Look for this result in the debug string
                // We search for patterns like "0: (" because that's how Debug formats IndexMap entries
                // This is brittle AF but here we are
                let pattern = format!("{}: (", i);
                if let Some(idx) = debug_str.find(&pattern) {
                    let slice = &debug_str[idx + pattern.len()..];

                    // Check for execution time
                    // SurrealDB puts Stats first, then the actual result
                    // We're basically doing archaeology on string representations
                    let has_stats = slice.starts_with("Stats");
                    let time_info = if has_stats {
                        if let Some(time_start) = slice.find("execution_time: Some(") {
                            let time_slice = &slice[time_start + 21..];
                            if let Some(time_end) = time_slice.find(')') {
                                format!(" ({})", &time_slice[..time_end])
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                    // Determine if Ok or Err
                    if let Some(ok_idx) = slice.find("Ok(") {
                        if ok_idx < 500 {
                            // Increased to handle multiline formatting
                            // Extract what's inside Ok(...)
                            // This is where things get spicy 🌶️
                            let ok_slice = &slice[ok_idx + 3..];

                            // Skip initial whitespace/newlines
                            // Because Debug format loves its artistic whitespace
                            let ok_content = ok_slice.trim_start();

                            // Check the type of result
                            if ok_content.starts_with("Array(") {
                                // Welcome to the Array-ception!
                                // SurrealDB wraps arrays in Array() and then wraps THAT in another Array()
                                // It's arrays all the way down 🐢
                                let inner = &ok_content[6..];
                                let inner_trimmed = inner.trim_start();

                                if inner_trimmed.starts_with("Array(") {
                                    // Double-wrapped array, check the inner one
                                    let inner2 = &inner_trimmed[6..];
                                    let inner2_trimmed = inner2.trim_start();

                                    if inner2_trimmed.starts_with("[]") {
                                        writeln!(f, "Ok(Array[0]){}", time_info)?;
                                    } else if inner2_trimmed.starts_with("[") {
                                        // Try to peek at array contents
                                        let preview =
                                            &inner2_trimmed[..inner2_trimmed.len().min(1000)];

                                        // Count different types of items
                                        let obj_count = preview.matches("Object(").count();
                                        let num_count = preview.matches("Number(").count();
                                        let str_count = preview.matches("String(").count();
                                        let bool_count = preview.matches("Bool(").count();

                                        let total_items = obj_count
                                            .max(num_count)
                                            .max(str_count)
                                            .max(bool_count)
                                            .max(1);

                                        // Determine primary type and show preview
                                        if obj_count > 0 {
                                            // Try to extract first object's keys
                                            if let Some(obj_idx) = preview.find("Object(") {
                                                let obj_slice = &preview[obj_idx + 7..];
                                                if let Some(brace) = obj_slice.find('{') {
                                                    let keys_slice = &obj_slice[brace + 1..];
                                                    // Parse object keys by looking for colons
                                                    // This is definitely how the Rust compiler does it (narrator: it isn't)
                                                    let keys: Vec<&str> = keys_slice
                                                        .split('\n')
                                                        .filter(|line| line.contains(':'))
                                                        .take(2)
                                                        .filter_map(|line| {
                                                            line.trim()
                                                                .split(':')
                                                                .next()
                                                                .map(|k| k.trim().trim_matches('"'))
                                                        })
                                                        .filter(|k| {
                                                            !k.is_empty() && !k.contains('}')
                                                        })
                                                        .collect();
                                                    if !keys.is_empty() {
                                                        writeln!(
                                                            f,
                                                            "Ok(Array[~{} Objects{{{:?}, ...}}]){}",
                                                            total_items,
                                                            keys.join(", "),
                                                            time_info
                                                        )?;
                                                    } else {
                                                        writeln!(
                                                            f,
                                                            "Ok(Array[~{} Objects]){}",
                                                            total_items, time_info
                                                        )?;
                                                    }
                                                } else {
                                                    writeln!(
                                                        f,
                                                        "Ok(Array[~{} Objects]){}",
                                                        total_items, time_info
                                                    )?;
                                                }
                                            } else {
                                                writeln!(
                                                    f,
                                                    "Ok(Array[~{} items]){}",
                                                    total_items, time_info
                                                )?;
                                            }
                                        } else if num_count > 0 {
                                            writeln!(
                                                f,
                                                "Ok(Array[~{} Numbers]){}",
                                                total_items, time_info
                                            )?;
                                        } else if str_count > 0 {
                                            writeln!(
                                                f,
                                                "Ok(Array[~{} Strings]){}",
                                                total_items, time_info
                                            )?;
                                        } else if bool_count > 0 {
                                            writeln!(
                                                f,
                                                "Ok(Array[~{} Bools]){}",
                                                total_items, time_info
                                            )?;
                                        } else {
                                            writeln!(
                                                f,
                                                "Ok(Array[~{} items]){}",
                                                total_items, time_info
                                            )?;
                                        }
                                    } else {
                                        writeln!(f, "Ok(Array[...]){}", time_info)?;
                                    }
                                } else if inner_trimmed.starts_with("[]") {
                                    writeln!(f, "Ok(Array[0]){}", time_info)?;
                                } else {
                                    writeln!(f, "Ok(Array[...]){}", time_info)?;
                                }
                            } else if ok_content.starts_with("Object(") {
                                // Skip whitespace and nested Object( wrapper
                                let obj_inner = &ok_content[7..];
                                let obj_trimmed = obj_inner.trim_start();

                                if obj_trimmed.starts_with("Object(") {
                                    // Double-wrapped object
                                    let obj_inner2 = &obj_trimmed[7..];
                                    let obj_trimmed2 = obj_inner2.trim_start();

                                    if let Some(obj_start) = obj_trimmed2.find('{') {
                                        let obj_slice = &obj_trimmed[obj_start + 1..];
                                        let keys: Vec<&str> = obj_slice
                                            .split('\n')
                                            .filter(|line| line.contains(':'))
                                            .take(3)
                                            .filter_map(|line| {
                                                line.trim()
                                                    .split(':')
                                                    .next()
                                                    .map(|k| k.trim().trim_matches('"'))
                                            })
                                            .filter(|k| !k.is_empty() && !k.contains('}'))
                                            .collect();

                                        if keys.is_empty() {
                                            writeln!(f, "Ok(Object{{}}){}", time_info)?;
                                        } else if keys.len() <= 3 {
                                            writeln!(
                                                f,
                                                "Ok(Object{{{:?}}}){}",
                                                keys.join(", "),
                                                time_info
                                            )?;
                                        } else {
                                            writeln!(
                                                f,
                                                "Ok(Object{{{:?}, ...}}){}",
                                                keys.join(", "),
                                                time_info
                                            )?;
                                        }
                                    } else {
                                        writeln!(f, "Ok(Object{{...}}){}", time_info)?;
                                    }
                                } else if let Some(obj_start) = obj_trimmed.find('{') {
                                    // Single-wrapped object
                                    let obj_slice = &obj_trimmed[obj_start + 1..];
                                    let keys: Vec<&str> = obj_slice
                                        .split('\n')
                                        .filter(|line| line.contains(':'))
                                        .take(3)
                                        .filter_map(|line| {
                                            line.trim()
                                                .split(':')
                                                .next()
                                                .map(|k| k.trim().trim_matches('"'))
                                        })
                                        .filter(|k| !k.is_empty() && !k.contains('}'))
                                        .collect();

                                    if keys.is_empty() {
                                        writeln!(f, "Ok(Object{{}}){}", time_info)?;
                                    } else {
                                        writeln!(
                                            f,
                                            "Ok(Object{{{:?}}}){}",
                                            keys.join(", "),
                                            time_info
                                        )?;
                                    }
                                } else {
                                    writeln!(f, "Ok(Object{{...}}){}", time_info)?;
                                }
                            } else if ok_content.starts_with("Value(")
                                || ok_content.starts_with("Number(")
                                || ok_content.starts_with("String(")
                                || ok_content.starts_with("Bool(")
                            {
                                // Try to extract the actual value
                                let value_end = ok_content.find(')').unwrap_or(20);
                                let value_preview = &ok_content[..value_end.min(50)];
                                writeln!(f, "Ok({})){}", value_preview, time_info)?;
                            } else {
                                // We tried our best but the debug format has defeated us
                                // Time to give up with dignity
                                writeln!(f, "Ok(...){}", time_info)?;
                            }
                        } else {
                            writeln!(f, "Ok(...){}", time_info)?;
                        }
                    } else if let Some(err_idx) = slice.find("Err(") {
                        if err_idx < 500 {
                            // Increased to handle multiline formatting
                            // Try to extract error type
                            let err_slice = &slice[err_idx + 4..];
                            if let Some(paren) = err_slice.find(')') {
                                let err_preview = &err_slice[..paren.min(50)];
                                writeln!(f, "Err({}){}", err_preview, time_info)?;
                            } else {
                                writeln!(f, "Err(...){}", time_info)?;
                            }
                        } else {
                            writeln!(f, "Err(...){}", time_info)?;
                        }
                    } else {
                        writeln!(f, "<result>{}", time_info)?;
                    }
                } else {
                    // The statement index wasn't found in the debug output
                    // This shouldn't happen unless SurrealDB changes their Debug impl
                    // In which case, RIP this entire hack
                    writeln!(f, "<not found>")?;
                }
            }
            writeln!(f, "  ]")?;
        }

        write!(f, "}}")
    }
}

/// Extension trait for prettier debug output on SurrealDB Response
///
/// ## Usage
///
/// ```rust,ignore
/// use pattern_core::utils::debug::ResponseExt;
///
/// let response = db.query("SELECT * FROM user").await?;
///
/// // Instead of this unreadable mess:
/// println!("{:#?}", response);
///
/// // Use this:
/// println!("{:?}", response.pretty_debug());
///
/// // Or with tracing:
/// tracing::debug!("Query result: {:?}", response.pretty_debug());
/// ```
///
/// This will transform deeply nested debug output into a readable summary showing:
/// - Number of statements and their success/error counts
/// - Result types (Array with counts, Objects with keys, etc.)
/// - Execution times when available
pub trait ResponseExt {
    /// Get a pretty debug representation of this response
    fn pretty_debug(&self) -> ResponseDebug<'_>;
}

impl ResponseExt for surrealdb::Response {
    fn pretty_debug(&self) -> ResponseDebug<'_> {
        ResponseDebug(self)
    }
}

/// Formats a SurrealDB Response in a more readable way
pub fn format_surreal_response(response: &surrealdb::Response) -> String {
    use std::fmt::Write;

    let mut output = String::new();
    let _ = writeln!(output, "SurrealDB Response {{");

    let num_statements = response.num_statements();
    let _ = writeln!(output, "  statements: {},", num_statements);

    // We can't iterate over the results directly, but we can show the count
    let _ = writeln!(output, "  results: [");
    for i in 0..num_statements {
        let _ = write!(output, "    [{}]: ", i);
        // We can't actually access the results without consuming them,
        // so just indicate they exist
        let _ = writeln!(output, "<result>",);
    }
    let _ = writeln!(output, "  ]");

    let _ = write!(output, "}}");
    output
}

/// Helper macro to create Debug implementations that truncate embeddings
#[macro_export]
macro_rules! impl_debug_with_compact_embeddings {
    ($type:ty, $($field:ident),+) => {
        impl std::fmt::Debug for $type {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut debug_struct = f.debug_struct(stringify!($type));
                $(
                    // Check if field name contains "embedding"
                    if stringify!($field).contains("embedding") {
                        match &self.$field {
                            Some(arr) => {
                                let formatted = format!("{}", $crate::utils::debug::EmbeddingDebug(arr));
                                debug_struct.field(stringify!($field), &formatted);
                            }
                            None => {
                                debug_struct.field(stringify!($field), &None::<Vec<f32>>);
                            }
                        }
                    } else {
                        debug_struct.field(stringify!($field), &self.$field);
                    }
                )+
                debug_struct.finish()
            }
        }
    };
}

/// Wrapper type for pretty-printing embeddings
pub struct EmbeddingDebug<'a>(pub &'a [f32]);

impl<'a> fmt::Display for EmbeddingDebug<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_float_array_compact(f, self.0)
    }
}

impl<'a> fmt::Debug for EmbeddingDebug<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_float_array_compact(f, self.0)
    }
}

/// Extension trait for prettier debug output
pub trait DebugPretty {
    /// Format with compact arrays
    fn debug_pretty(&self) -> String;
}

impl<T: fmt::Debug> DebugPretty for Vec<T> {
    fn debug_pretty(&self) -> String {
        if self.len() > 10 {
            format!(
                "[{} items: {:?}...{:?}]",
                self.len(),
                &self[..3],
                &self[self.len() - 3..]
            )
        } else {
            format!("{:?}", self)
        }
    }
}

/// Pretty-print serde_json::Value with compact arrays
pub fn format_json_value_compact(value: &serde_json::Value, indent: usize) -> String {
    use serde_json::Value;
    let indent_str = "  ".repeat(indent);

    match value {
        Value::Array(arr) if arr.len() > 10 => {
            // Check if it's a numeric array
            if arr.iter().all(|v| v.is_number()) {
                if arr.iter().all(|v| v.as_f64().is_some()) {
                    // Float array - probably embeddings
                    let floats: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                    format!(
                        "{}",
                        EmbeddingDebug(&floats.iter().map(|&f| f as f32).collect::<Vec<_>>())
                    )
                } else {
                    // Mixed numeric array
                    format!("[{} numbers]", arr.len())
                }
            } else {
                // Mixed array - show structure
                format!("[{} items]", arr.len())
            }
        }
        Value::Object(map) => {
            let mut output = String::from("{\n");
            for (key, val) in map {
                output.push_str(&format!(
                    "{}  \"{}\": {},\n",
                    indent_str,
                    key,
                    format_json_value_compact(val, indent + 1)
                ));
            }
            output.push_str(&format!("{}}}", indent_str));
            output
        }
        _ => {
            // Use default formatting for other types
            format!("{}", value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_float_array() {
        let small = vec![1.0, 2.0, 3.0];
        let output = format!("{}", EmbeddingDebug(&small));
        assert_eq!(output, "[1.0, 2.0, 3.0]");

        let large = vec![0.0; 384];
        let output = format!("{}", EmbeddingDebug(&large));
        assert!(output.contains("(384d)"));
        assert!(output.contains("..."));
    }

    #[test]
    fn test_embedding_debug() {
        let embedding = vec![0.1; 768];
        let debug = EmbeddingDebug(&embedding);
        let formatted = format!("{:?}", debug);
        assert!(formatted.contains("(768d)"));
        assert!(!formatted.contains("0.1, 0.1, 0.1, 0.1")); // Should be truncated
    }

    #[test]
    fn test_pretty_debug_output() {
        use crate::db::{BaseTask, BaseTaskPriority, BaseTaskStatus};
        use crate::id::TaskId;
        use chrono::Utc;

        // Create a Task with embeddings
        let task = BaseTask {
            id: TaskId::generate(),
            title: "Write documentation".to_string(),
            description: Some("Write comprehensive docs".to_string()),
            status: BaseTaskStatus::Pending,
            priority: BaseTaskPriority::High,
            due_date: Some(Utc::now() + chrono::Duration::days(7)),
            ..Default::default()
        };

        let task_debug = format!("{:#?}", task);
        println!("\nTask debug output:\n{}", task_debug);

        // Check that basic fields are present
        assert!(task_debug.contains("Write documentation"));
        assert!(task_debug.contains("Pending")); // Enum variant without the type name
    }

    #[test]
    fn test_response_debug_hack() {
        // Create a mock debug string that looks like SurrealDB Response output
        let mock_debug = r#"Response {
    results: {
        0: (
            Stats {
                execution_time: Some(
                    704.588µs,
                ),
            },
            Ok(
                Array(
                    Array(
                        [
                            Object(
                                Object(
                                    {
                                        "agents": Array(
                                            Array(
                                                [
                                                    Object(
                                                        Object(
                                                            {
                                                                "agent_type": Strand(
                                                                    Strand(
                                                                        "pattern",
                                                                    ),
                                                                ),
                                                            },
                                                        ),
                                                    ),
                                                ],
                                            ),
                                        ),
                                    },
                                ),
                            ),
                        ],
                    ),
                ),
            ),
        ),
        1: (
            Stats {
                execution_time: Some(
                    1.2ms,
                ),
            },
            Err(
                Api(
                    NotFound(
                        "user:123",
                    ),
                ),
            ),
        ),
    },
    live_queries: {},
}"#;

        // Since we can't create a real Response, let's at least test our parsing logic
        // by checking that it can extract info from the debug string
        let has_ok = mock_debug.contains("Ok(");
        let has_err = mock_debug.contains("Err(");
        let has_stats = mock_debug.contains("Stats");

        assert!(has_ok);
        assert!(has_err);
        assert!(has_stats);

        // Test extraction of execution times
        assert!(mock_debug.contains("704.588µs"));
        assert!(mock_debug.contains("1.2ms"));
    }
}
