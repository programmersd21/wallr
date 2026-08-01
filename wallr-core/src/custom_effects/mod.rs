//! Sandboxed declarative custom-effect validation and WGSL transpilation.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A declarative per-pixel effect definition.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CustomEffect {
    /// Effect parameters and their default numeric values.
    #[serde(default)]
    pub params: HashMap<String, serde_yaml::Value>,
    /// Sandboxed expression body returning a color expression.
    pub field: String,
}

/// Error returned when a custom field cannot be safely transpiled.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CustomEffectError {
    #[error("custom effect field is empty")]
    Empty,
    #[error("unknown function '{0}'")]
    UnknownFunction(String),
    #[error("undefined variable '{0}'")]
    UndefinedVariable(String),
    #[error("unsupported custom-effect construct '{0}'")]
    Unsupported(String),
}

const FUNCTIONS: &[&str] = &[
    "mix",
    "clamp",
    "floor",
    "ceil",
    "length",
    "hash",
    "noise",
    "smoothstep",
    "sin",
    "cos",
    "tan",
    "abs",
    "min",
    "max",
    "pow",
    "direction_vector",
    "sample",
    "vec2",
];
const CONTEXT: &[&str] = &["t", "uv", "resolution", "old", "new", "time_absolute", "pi"];

/// Validate and transpile a field into a WGSL expression body.
pub fn transpile(name: &str, effect: &CustomEffect) -> Result<String, CustomEffectError> {
    if effect.field.trim().is_empty() {
        return Err(CustomEffectError::Empty);
    }
    let tokens = tokenize(&effect.field);
    let mut declared: HashSet<String> = effect.params.keys().cloned().collect();
    declared.extend(CONTEXT.iter().map(|s| s.to_string()));
    for (index, token) in tokens.iter().enumerate() {
        if !is_identifier(token) {
            continue;
        }
        let is_call = token == "vec2"
            || tokens
                .get(index + 1)
                .is_some_and(|next| next == "(" || next == "<");
        if is_call {
            if !FUNCTIONS.contains(&token.as_str())
                && token != "f32"
                && token != "return"
                && token != "let"
            {
                return Err(CustomEffectError::UnknownFunction(token.clone()));
            }
        } else if !declared.contains(token)
            && token != "return"
            && token != "let"
            && token != "color"
            && token != "shard"
            && token != "delay"
            && token != "local_t"
            && token != "offset"
            && token != "alpha"
            && token != "f32"
        {
            // Assignment names are declared by the simple `name = expression` form.
            let assignment = tokens.get(index + 1).is_some_and(|next| next == "=");
            if assignment {
                declared.insert(token.clone());
            } else {
                return Err(CustomEffectError::UndefinedVariable(token.clone()));
            }
        }
    }
    if effect.field.contains('{')
        || effect.field.contains('}')
        || effect.field.contains("for ")
        || effect.field.contains("while ")
    {
        return Err(CustomEffectError::Unsupported(
            "loops or block delimiters".into(),
        ));
    }
    Ok(format!("// custom effect: {name}\n{}", effect.field))
}

fn tokenize(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in source.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            current.push(c);
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            if c == '(' || c == '=' {
                tokens.push(c.to_string());
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}
fn is_identifier(token: &str) -> bool {
    token
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transpiles_valid_field() {
        let effect = CustomEffect {
            params: HashMap::new(),
            field: "return mix(old, new, t)".into(),
        };
        assert!(transpile("fade", &effect).is_ok());
    }
    #[test]
    fn rejects_unknown_names() {
        let effect = CustomEffect {
            params: HashMap::new(),
            field: "return explode(old)".into(),
        };
        assert!(matches!(
            transpile("bad", &effect),
            Err(CustomEffectError::UnknownFunction(_))
        ));
    }
}
