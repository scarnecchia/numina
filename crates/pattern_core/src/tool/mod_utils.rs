use serde_json::Value;

/// Inject a `request_heartbeat` boolean into a JSON Schema-like object if possible.
/// This is best-effort and tolerant of schema shape differences.
pub fn inject_request_heartbeat(schema: &mut Value) {
    let prop = (
        "request_heartbeat",
        Value::Object(
            [
                ("type".to_string(), Value::String("boolean".to_string())),
                (
                    "description".to_string(),
                    Value::String(
                        "Request a heartbeat continuation after tool execution".to_string(),
                    ),
                ),
                ("default".to_string(), Value::Bool(false)),
            ]
            .into_iter()
            .collect(),
        ),
    );

    // Try common shapes: { properties: {} } or nested under { schema: { properties: {} } }
    if let Some(obj) = schema.as_object_mut() {
        if let Some(props) = obj.get_mut("properties").and_then(|v| v.as_object_mut()) {
            props.insert(prop.0.to_string(), prop.1);
            return;
        }
        if let Some(schema_obj) = obj.get_mut("schema").and_then(|v| v.as_object_mut()) {
            if let Some(props) = schema_obj
                .get_mut("properties")
                .and_then(|v| v.as_object_mut())
            {
                props.insert(prop.0.to_string(), prop.1);
                return;
            }
        }
    }
}
