use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/");

    // Calculate hash of all entity definitions
    let mut hasher = DefaultHasher::new();

    // Hash the entity macro itself
    if let Ok(macro_content) = fs::read_to_string("../pattern_macros/src/lib.rs") {
        macro_content.hash(&mut hasher);
    }

    // Hash all files that likely contain entity definitions
    let entity_files = [
        "src/memory.rs",
        "src/users.rs",
        "src/agent/entity.rs",
        "src/message.rs",
        "src/db/entity.rs",
        "src/db/schema.rs",
        "src/oauth.rs",
        "src/atproto_identity.rs",
        "src/discord_identity.rs",
    ];

    for file in &entity_files {
        if let Ok(content) = fs::read_to_string(file) {
            // Only hash lines containing Entity derive or entity attributes
            for line in content.lines() {
                if line.contains("#[derive") && line.contains("Entity") {
                    line.hash(&mut hasher);
                } else if line.contains("#[entity") {
                    line.hash(&mut hasher);
                } else if line.contains("pub struct") || line.contains("pub enum") {
                    // Hash type definitions that might be entities
                    line.hash(&mut hasher);
                }
            }
        }
    }

    let schema_hash = hasher.finish();

    // Write the hash to a file that can be included at compile time
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let schema_hash_path = Path::new(&out_dir).join("schema_hash.txt");
    fs::write(schema_hash_path, schema_hash.to_string()).unwrap();

    println!("cargo:rustc-env=PATTERN_SCHEMA_HASH={}", schema_hash);
}
