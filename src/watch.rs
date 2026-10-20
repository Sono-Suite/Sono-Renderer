use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub const DEFAULT_ENTITY_MEMORY_LENGTH: i32 = 64;
pub const DEFAULT_ENTITY_DATA_LENGTH: i32 = 32;
pub const DEFAULT_ENTITY_SHARED_MEMORY_LENGTH: i32 = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchData {
    #[serde(
        default = "default_entity_memory_length",
        rename = "entityMemoryLength",
        alias = "entity_memory_length"
    )]
    pub entity_memory_length: i32,
    #[serde(
        default = "default_entity_data_length",
        rename = "entityDataLength",
        alias = "entity_data_length"
    )]
    pub entity_data_length: i32,
    #[serde(
        default = "default_entity_shared_memory_length",
        rename = "entitySharedMemoryLength",
        alias = "entity_shared_memory_length"
    )]
    pub entity_shared_memory_length: i32,
    #[serde(default)]
    pub archetypes: Vec<WatchArchetype>,
    #[serde(default)]
    /// Immutable graph storage is shared by renderer workers and GUI
    /// precompilation without copying the complete Watch graph.
    pub nodes: Arc<Vec<EngineNode>>,
    #[serde(default, deserialize_with = "deserialize_skin")]
    pub skin: Value,
    #[serde(default)]
    pub effect: Value,
    #[serde(default)]
    pub particle: Value,
    #[serde(default, rename = "updateSpawn", alias = "update_spawn")]
    pub update_spawn: Option<usize>,
    #[serde(default)]
    pub buckets: Vec<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn default_entity_memory_length() -> i32 {
    DEFAULT_ENTITY_MEMORY_LENGTH
}

fn default_entity_data_length() -> i32 {
    DEFAULT_ENTITY_DATA_LENGTH
}

fn default_entity_shared_memory_length() -> i32 {
    DEFAULT_ENTITY_SHARED_MEMORY_LENGTH
}

fn deserialize_skin<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Value, D::Error> {
    let skin = Value::deserialize(deserializer)?;
    parse_skin_render_mode(&skin)
        .map_err(|error| serde::de::Error::custom(format!("skin.renderMode: {error}")))?;
    Ok(skin)
}

fn parse_skin_render_mode(
    skin: &Value,
) -> Result<crate::skin_render_mode::SkinRenderModePreference, serde_json::Error> {
    match skin.get("renderMode") {
        None => Ok(crate::skin_render_mode::SkinRenderModePreference::Default),
        Some(value) => serde_json::from_value(value.clone()),
    }
}

impl WatchData {
    pub fn skin_render_mode(
        &self,
    ) -> Result<crate::skin_render_mode::SkinRenderModePreference, serde_json::Error> {
        parse_skin_render_mode(&self.skin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_memory_lengths_default_parse_and_preserve_unknown_fields() {
        let legacy: WatchData = serde_json::from_value(serde_json::json!({
            "archetypes": [], "nodes": []
        }))
        .unwrap();
        assert_eq!(legacy.entity_memory_length, 64);
        assert_eq!(legacy.entity_data_length, 32);
        assert_eq!(legacy.entity_shared_memory_length, 32);

        let explicit: WatchData = serde_json::from_value(serde_json::json!({
            "entityMemoryLength": 64,
            "entityDataLength": 32,
            "entitySharedMemoryLength": 32,
            "futureWatchField": {"version": 2},
            "nodes": []
        }))
        .unwrap();
        assert_eq!(explicit.entity_memory_length, legacy.entity_memory_length);
        assert_eq!(explicit.entity_data_length, legacy.entity_data_length);
        assert_eq!(
            explicit.entity_shared_memory_length,
            legacy.entity_shared_memory_length
        );
        assert_eq!(
            explicit.extra.get("futureWatchField"),
            Some(&serde_json::json!({"version": 2}))
        );
        let serialized = serde_json::to_value(explicit).unwrap();
        assert_eq!(serialized["entityMemoryLength"], 64);
        assert_eq!(serialized["entityDataLength"], 32);
        assert_eq!(serialized["entitySharedMemoryLength"], 32);
        assert!(serialized.get("entity_memory_length").is_none());
    }

    #[test]
    fn entity_memory_lengths_accept_zero_and_reject_malformed_or_out_of_i32_values() {
        let zero: WatchData = serde_json::from_value(serde_json::json!({
            "entityMemoryLength": 0,
            "entityDataLength": 0,
            "entitySharedMemoryLength": 0
        }))
        .unwrap();
        assert_eq!(zero.entity_memory_length, 0);
        assert_eq!(zero.entity_data_length, 0);
        assert_eq!(zero.entity_shared_memory_length, 0);

        for value in [
            serde_json::json!({"entityDataLength": "32"}),
            serde_json::json!({"entitySharedMemoryLength": 1.5}),
            serde_json::json!({"entityDataLength": 2147483648_i64}),
        ] {
            assert!(serde_json::from_value::<WatchData>(value).is_err());
        }
        let negative: WatchData = serde_json::from_value(serde_json::json!({
            "entityMemoryLength": -1
        }))
        .unwrap();
        assert!(crate::watch_runtime::WatchRuntime::new(
            &negative,
            &serde_json::from_value(serde_json::json!({"entities":[]})).unwrap()
        )
        .is_err());
    }
}
/// A compiled Sonolus node. `func` is deliberately an open string: engines may
/// use any operation name, even when this renderer has no VM implementation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineNode {
    #[serde(default)]
    pub func: Option<String>,
    #[serde(default)]
    pub args: Vec<Value>,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchArchetype {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub imports: Vec<Value>,
    #[serde(default)]
    pub exports: Vec<Value>,
    #[serde(default)]
    #[serde(rename = "hasInput", alias = "has_input")]
    pub has_input: bool,
    #[serde(default)]
    pub preprocess: Option<Value>,
    #[serde(default, rename = "spawnTime", alias = "spawn_time")]
    pub spawn_time: Option<Value>,
    #[serde(default, rename = "despawnTime", alias = "despawn_time")]
    pub despawn_time: Option<Value>,
    #[serde(default)]
    pub initialize: Option<Value>,
    #[serde(default, rename = "updateSequential", alias = "update_sequential")]
    pub update_sequential: Option<Value>,
    #[serde(default, rename = "updateParallel", alias = "update_parallel")]
    pub update_parallel: Option<Value>,
    #[serde(default)]
    pub terminate: Option<Value>,
    #[serde(flatten)]
    pub callbacks: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WatchInventory {
    pub archetypes: Vec<String>,
    pub functions: Vec<String>,
    pub memory_blocks: Vec<String>,
    pub skin_bindings: Value,
    pub effect_bindings: Value,
    pub particle_bindings: Value,
}

pub fn inventory(data: &WatchData) -> WatchInventory {
    let mut functions = Vec::new();
    let mut memory = Vec::new();
    fn walk(v: &Value, functions: &mut Vec<String>, memory: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                for key in ["name", "function", "operator", "type"] {
                    if let Some(s) = o.get(key).and_then(Value::as_str) {
                        if ["function", "operator"].contains(&key)
                            || (key == "type" && o.contains_key("args"))
                            || (key == "name"
                                && (o.contains_key("args")
                                    || o.contains_key("arguments")
                                    || o.contains_key("inputs")))
                        {
                            functions.push(s.to_string());
                        }
                    }
                }
                for (k, val) in o {
                    if k.to_ascii_lowercase().contains("memory")
                        || k.to_ascii_lowercase().contains("block")
                    {
                        if let Some(s) = val.as_str() {
                            memory.push(s.into())
                        } else if val.is_number() {
                            memory.push(val.to_string())
                        }
                    }
                    walk(val, functions, memory);
                }
            }
            Value::Array(a) => {
                for x in a {
                    walk(x, functions, memory)
                }
            }
            _ => {}
        }
    }
    for node in data.nodes.iter() {
        if let Some(func) = &node.func {
            functions.push(func.clone());
            if [
                "Get",
                "Set",
                "GetPointed",
                "SetPointed",
                "GetShifted",
                "SetShifted",
            ]
            .contains(&func.as_str())
            {
                if let Some(block_arg) = node
                    .args
                    .first()
                    .and_then(Value::as_u64)
                    .and_then(|i| data.nodes.get(i as usize))
                {
                    if let Some(block_id) = block_arg.value.as_ref() {
                        memory.push(block_id.to_string());
                    }
                }
            }
        }
        walk(
            &Value::Object(
                node.extra
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .chain(
                        node.value
                            .iter()
                            .map(|v| (String::from("value"), v.clone())),
                    )
                    .collect(),
            ),
            &mut functions,
            &mut memory,
        );
    }
    walk(
        &serde_json::to_value(&data.archetypes).unwrap_or(Value::Null),
        &mut functions,
        &mut memory,
    );
    functions.sort();
    functions.dedup();
    memory.sort();
    memory.dedup();
    WatchInventory {
        archetypes: data.archetypes.iter().map(|a| a.name.clone()).collect(),
        functions,
        memory_blocks: memory,
        skin_bindings: data.skin.clone(),
        effect_bindings: data.effect.clone(),
        particle_bindings: data.particle.clone(),
    }
}
