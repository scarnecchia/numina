//! Test to see generated schemas

#[cfg(test)]
mod tests {
    use crate::tool::builtin::send_message::SendMessageInput;
    use crate::tool::builtin::{
        ArchivalMemoryOperationType, CoreMemoryOperationType, ManageArchivalMemoryInput,
        ManageCoreMemoryInput, MessageTarget, TargetType,
    };
    use schemars::schema_for;

    #[test]
    fn test_message_target_schema() {
        let schema = schema_for!(MessageTarget);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        println!("MessageTarget schema:\n{}", json);
    }

    #[test]
    fn test_target_type_schema() {
        let schema = schema_for!(TargetType);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        println!("TargetType schema:\n{}", json);

        // Check if it contains oneOf
        assert!(
            !json.contains("oneOf"),
            "TargetType should not generate oneOf schema"
        );
    }

    #[test]
    fn test_send_message_input_schema() {
        let schema = schema_for!(SendMessageInput);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        println!("SendMessageInput schema:\n{}", json);
    }

    #[test]
    fn test_core_memory_operation_type_schema() {
        let schema = schema_for!(CoreMemoryOperationType);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        println!("CoreMemoryOperationType schema:\n{}", json);

        // Check if it contains oneOf
        if json.contains("oneOf") {
            eprintln!("WARNING: CoreMemoryOperationType generates oneOf schema!");
            eprintln!("This will cause issues with Gemini API");
        }
    }

    #[test]
    fn test_manage_core_memory_input_schema() {
        let schema = schema_for!(ManageCoreMemoryInput);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        println!("ManageCoreMemoryInput schema:\n{}", json);

        // Check for problematic patterns
        if json.contains("oneOf") {
            eprintln!("WARNING: ManageCoreMemoryInput contains oneOf!");
        }
        if json.contains("const") {
            eprintln!("WARNING: ManageCoreMemoryInput contains const!");
        }
    }

    #[test]
    fn test_archival_memory_operation_type_schema() {
        let schema = schema_for!(ArchivalMemoryOperationType);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        println!("ArchivalMemoryOperationType schema:\n{}", json);

        // Check if it contains oneOf
        if json.contains("oneOf") {
            eprintln!("WARNING: ArchivalMemoryOperationType generates oneOf schema!");
            eprintln!("This will cause issues with Gemini API");
        }
    }

    #[test]
    fn test_manage_archival_memory_input_schema() {
        let schema = schema_for!(ManageArchivalMemoryInput);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        println!("ManageArchivalMemoryInput schema:\n{}", json);

        // Check for problematic patterns
        if json.contains("oneOf") {
            eprintln!("WARNING: ManageArchivalMemoryInput contains oneOf!");
        }
        if json.contains("const") {
            eprintln!("WARNING: ManageArchivalMemoryInput contains const!");
        }
    }
}
