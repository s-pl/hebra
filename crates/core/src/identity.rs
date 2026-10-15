//! Who is acting, and why: resolved from the environment, the workspace and
//! the repository config.
//!
//! | Variable          | Meaning                                         |
//! |-------------------|-------------------------------------------------|
//! | `HB_ACTOR`        | Name of the actor                               |
//! | `HB_ACTOR_KIND`   | `human` or `agent`                              |
//! | `HB_ON_BEHALF_OF` | Person an agent works for                       |
//! | `HB_TASK`         | Task the change belongs to                      |
//! | `HB_SESSION`      | Agent session id                                |
//! | `HB_PROMPT`       | Prompt that produced the change                 |
//!
//! Known agents are detected automatically (for example Claude Code sets
//! `CLAUDECODE=1`).

use crate::model::{Actor, ActorKind, Provenance, WorkspaceInfo};

/// Environment variables set by coding agents, and the name they map to.
const KNOWN_AGENTS: &[(&str, &str)] = &[("CLAUDECODE", "claude-code")];

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// Resolves the acting identity. Precedence: explicit environment, then a
/// detected agent, then the workspace's agent, then the configured user.
pub fn resolve_actor(
    env: impl Fn(&str) -> Option<String>,
    workspace: Option<&WorkspaceInfo>,
    user: Option<String>,
) -> Actor {
    let user = non_empty(user)
        .or_else(|| non_empty(env("USER")))
        .or_else(|| non_empty(env("USERNAME")))
        .unwrap_or_else(|| "unknown".to_owned());
    let kind_env = non_empty(env("HB_ACTOR_KIND"));
    let on_behalf_of = non_empty(env("HB_ON_BEHALF_OF"));

    if let Some(name) = non_empty(env("HB_ACTOR")) {
        let kind = match kind_env.as_deref() {
            Some("agent") => ActorKind::Agent,
            _ if on_behalf_of.is_some() => ActorKind::Agent,
            _ => ActorKind::Human,
        };
        let on_behalf_of = match kind {
            ActorKind::Agent => on_behalf_of.or(Some(user)),
            ActorKind::Human => None,
        };
        return Actor {
            name,
            kind,
            on_behalf_of,
        };
    }
    if let Some(actor) = workspace.and_then(|w| w.actor.clone()) {
        return actor;
    }
    if let Some((_, name)) = KNOWN_AGENTS
        .iter()
        .find(|(var, _)| non_empty(env(var)).is_some())
    {
        return Actor {
            name: (*name).to_owned(),
            kind: ActorKind::Agent,
            on_behalf_of: on_behalf_of.or(Some(user)),
        };
    }
    Actor::human(user)
}

/// Provenance from the environment, falling back to the workspace task.
pub fn provenance(
    env: impl Fn(&str) -> Option<String>,
    workspace: Option<&WorkspaceInfo>,
) -> Provenance {
    Provenance {
        task: non_empty(env("HB_TASK")).or_else(|| workspace.and_then(|w| w.task.clone())),
        session: non_empty(env("HB_SESSION")),
        prompt: non_empty(env("HB_PROMPT")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn explicit_agent_works_for_configured_user() {
        let actor = resolve_actor(
            env(&[("HB_ACTOR", "bot"), ("HB_ACTOR_KIND", "agent")]),
            None,
            Some("ana".into()),
        );
        assert_eq!(actor.kind, ActorKind::Agent);
        assert_eq!(actor.on_behalf_of.as_deref(), Some("ana"));
    }

    #[test]
    fn detects_claude_code() {
        let actor = resolve_actor(env(&[("CLAUDECODE", "1")]), None, Some("ana".into()));
        assert_eq!(actor.name, "claude-code");
        assert_eq!(actor.kind, ActorKind::Agent);
    }

    #[test]
    fn defaults_to_human_user() {
        let actor = resolve_actor(env(&[("USER", "luis")]), None, None);
        assert_eq!(actor, Actor::human("luis"));
    }
}
