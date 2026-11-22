use std::collections::BTreeMap;

use agent::{AcpFlavor, HarnessKind};

use crate::settings_store::{AgentSettings, CustomAgent};

pub const CUSTOM_PREFIX: &str = "custom:";
pub const DEFAULT_PROFILE: &str = "claude";

struct Builtin {
    id: &'static str,
    name: &'static str,
    kind: HarnessKind,
    command: &'static str,
    login: Option<LoginHint>,
}

const BUILTINS: [Builtin; 5] = [
    Builtin {
        id: DEFAULT_PROFILE,
        name: "Claude Code",
        kind: HarnessKind::Claude,
        command: "claude",
        login: Some(LoginHint::Command("claude auth login")),
    },
    Builtin {
        id: "codex",
        name: "Codex",
        kind: HarnessKind::Codex,
        command: "codex",
        login: Some(LoginHint::Command("codex login")),
    },
    Builtin {
        id: "opencode",
        name: "OpenCode",
        kind: HarnessKind::Acp(AcpFlavor::OpenCode),
        command: "opencode acp",
        login: Some(LoginHint::Command("opencode auth login")),
    },
    Builtin {
        id: "deepseek",
        name: "DeepSeek Harness",
        kind: HarnessKind::Acp(AcpFlavor::DeepSeek),
        command: "dsh acp",
        login: Some(LoginHint::EnvVar("DEEPSEEK_API_KEY")),
    },
    Builtin {
        id: "pi",
        name: "Pi",
        kind: HarnessKind::Pi,
        command: "pi",
        login: Some(LoginHint::Command("pi auth login")),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginHint {
    Command(&'static str),
    EnvVar(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub kind: HarnessKind,
    pub default_command: String,
    pub custom: bool,
    login: Option<LoginHint>,
}

impl Profile {
    fn builtin(builtin: &Builtin) -> Self {
        Self {
            id: builtin.id.to_string(),
            name: builtin.name.to_string(),
            kind: builtin.kind,
            default_command: builtin.command.to_string(),
            custom: false,
            login: builtin.login,
        }
    }

    fn custom(agent: &CustomAgent) -> Self {
        Self {
            id: format!("{CUSTOM_PREFIX}{}", agent.name),
            name: agent.name.clone(),
            kind: HarnessKind::Acp(AcpFlavor::Generic),
            default_command: agent.command.trim().to_string(),
            custom: true,
            login: None,
        }
    }

    pub fn has_sandbox_on(&self, windows: bool) -> bool {
        match self.kind {
            HarnessKind::Claude => !windows,
            HarnessKind::Codex => true,
            HarnessKind::Acp(_) | HarnessKind::Pi => false,
        }
    }

    pub fn has_sandbox(&self) -> bool {
        self.has_sandbox_on(cfg!(windows))
    }

    pub fn login_hint(&self) -> Option<LoginHint> {
        self.login
    }
}

fn is_usable<'a>(name: &str, command: &str, mut accepted: impl Iterator<Item = &'a str>) -> bool {
    !name.is_empty()
        && name.trim() == name
        && !command.trim().is_empty()
        && !BUILTINS
            .iter()
            .any(|builtin| builtin.id.eq_ignore_ascii_case(name))
        && !accepted.any(|taken| taken.eq_ignore_ascii_case(name))
}

fn non_blank(value: Option<&String>) -> Option<&str> {
    value.map(|v| v.trim()).filter(|v| !v.is_empty())
}

impl AgentSettings {
    pub fn profiles(&self) -> Vec<Profile> {
        let mut profiles: Vec<Profile> = BUILTINS.iter().map(Profile::builtin).collect();
        let mut taken: Vec<&str> = Vec::new();
        for agent in &self.custom {
            if is_usable(&agent.name, &agent.command, taken.iter().copied()) {
                taken.push(&agent.name);
                profiles.push(Profile::custom(agent));
            }
        }
        profiles
    }

    pub fn profile(&self, id: &str) -> Option<Profile> {
        self.profiles().into_iter().find(|profile| profile.id == id)
    }

    pub fn active_profile(&self) -> Profile {
        self.profile(&self.active)
            .or_else(|| self.profile(DEFAULT_PROFILE))
            .unwrap_or_else(|| Profile::builtin(&BUILTINS[0]))
    }

    pub fn command(&self, profile: &Profile) -> String {
        non_blank(self.commands.get(&profile.id))
            .unwrap_or(&profile.default_command)
            .to_string()
    }

    pub fn model(&self, profile: &Profile) -> Option<String> {
        non_blank(self.models.get(&profile.id)).map(str::to_string)
    }
}

pub fn set_override(map: &mut BTreeMap<String, String>, id: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        map.remove(id);
    } else {
        map.insert(id.to_string(), value.to_string());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedAgents {
    pub agents: Vec<CustomAgent>,
    pub ignored: Vec<String>,
}

pub fn parse_custom_agents(text: &str) -> ParsedAgents {
    let mut parsed = ParsedAgents::default();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let usable = line.split_once('=').and_then(|(name, command)| {
            let (name, command) = (name.trim(), command.trim());
            is_usable(
                name,
                command,
                parsed.agents.iter().map(|agent| agent.name.as_str()),
            )
            .then(|| CustomAgent {
                name: name.to_string(),
                command: command.to_string(),
            })
        });
        match usable {
            Some(agent) => parsed.agents.push(agent),
            None => parsed.ignored.push(line.to_string()),
        }
    }
    parsed
}

pub fn format_custom_agents(agents: &[CustomAgent]) -> String {
    agents
        .iter()
        .map(|agent| format!("{} = {}", agent.name, agent.command))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "agent_profiles_tests.rs"]
mod tests;
