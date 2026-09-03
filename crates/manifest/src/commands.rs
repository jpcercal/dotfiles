use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Root of `commands.yaml` — an ordered map of topic → curated command list,
/// seeded into the atuin history database by `dotfiles history seed`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct CommandsManifest(pub BTreeMap<String, CommandSection>);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandSection {
    pub description: String,
    pub commands: Vec<CommandEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandEntry {
    pub description: String,
    pub command: String,
}

impl CommandsManifest {
    pub fn sections(&self) -> impl Iterator<Item = (&String, &CommandSection)> {
        self.0.iter()
    }

    pub fn command_count(&self) -> usize {
        self.0.values().map(|s| s.commands.len()).sum()
    }
}
