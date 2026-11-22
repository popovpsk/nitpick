use std::collections::HashMap;

use agent::{HarnessKind, Model};
use gpui::Global;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub title: String,
    pub description: Option<String>,
}

pub fn choices(
    models: &[Model],
    stored: &str,
    default_label: &str,
    custom_label: &str,
) -> Vec<Choice> {
    let stored = stored.trim();
    let default_title = match models.iter().find(|model| model.is_default) {
        Some(model) => format!("{default_label} ({})", model.name),
        None => default_label.to_owned(),
    };
    let mut out = vec![Choice {
        value: String::new(),
        title: default_title,
        description: None,
    }];
    for model in models {
        if model.id.is_empty() {
            continue;
        }
        let title = if model.name == model.id {
            model.id.clone()
        } else {
            format!("{} ({})", model.name, model.id)
        };
        out.push(Choice {
            value: model.id.clone(),
            title,
            description: model.description.clone(),
        });
    }
    if !stored.is_empty() && !out.iter().any(|choice| choice.value == stored) {
        out.push(Choice {
            value: stored.to_owned(),
            title: stored.to_owned(),
            description: Some(custom_label.to_owned()),
        });
    }
    out
}

pub fn position(choices: &[Choice], stored: &str) -> usize {
    let stored = stored.trim();
    choices
        .iter()
        .position(|choice| choice.value == stored)
        .unwrap_or(0)
}

pub fn effective_command(typed: &str, default: &str) -> String {
    let typed = typed.trim();
    if typed.is_empty() { default } else { typed }.to_owned()
}

#[derive(Default)]
pub struct ModelCache(HashMap<(HarnessKind, String), Vec<Model>>);

impl Global for ModelCache {}

impl ModelCache {
    pub fn get(&self, kind: HarnessKind, command: &str) -> Option<&Vec<Model>> {
        self.0.get(&(kind, command.to_owned()))
    }

    pub fn put(&mut self, kind: HarnessKind, command: &str, models: Vec<Model>) {
        self.0.insert((kind, command.to_owned()), models);
    }
}

#[cfg(test)]
#[path = "agent_models_tests.rs"]
mod tests;
