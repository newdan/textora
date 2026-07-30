use serde::{Deserialize, Serialize};

pub const TEXTORA_MANAGED_BEGIN: &str = "// BEGIN TEXTORA MANAGED";
pub const TEXTORA_MANAGED_RULE: &str = "(?d).textora-save-*.tmp";
pub const TEXTORA_MANAGED_END: &str = "// END TEXTORA MANAGED";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedIgnoreState {
    Missing,
    Intact,
    Drifted,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IgnoreResponse {
    #[serde(default)]
    pub ignore: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct IgnoreRequest<'a> {
    pub ignore: &'a [String],
}

pub fn inspect_managed_ignore_rules(rules: &[String]) -> ManagedIgnoreState {
    let block = managed_block_positions(rules);
    if block.is_empty() {
        if rules.iter().any(|rule| rule == TEXTORA_MANAGED_RULE) {
            ManagedIgnoreState::Drifted
        } else {
            ManagedIgnoreState::Missing
        }
    } else if block.len() == 1
        && block[0].1 - block[0].0 == 3
        && rules[block[0].0] == TEXTORA_MANAGED_BEGIN
        && rules[block[0].0 + 1] == TEXTORA_MANAGED_RULE
        && rules[block[0].0 + 2] == TEXTORA_MANAGED_END
    {
        ManagedIgnoreState::Intact
    } else {
        ManagedIgnoreState::Drifted
    }
}

pub fn append_managed_ignore_block(rules: &[String]) -> Result<Vec<String>, crate::SyncError> {
    match inspect_managed_ignore_rules(rules) {
        ManagedIgnoreState::Intact => Ok(rules.to_vec()),
        ManagedIgnoreState::Drifted => Err(crate::SyncError::ConfigurationMismatch {
            operation: "validate managed ignore block",
        }),
        ManagedIgnoreState::Missing => {
            let mut updated = rules.to_vec();
            if updated.last().is_some_and(|rule| !rule.is_empty()) {
                updated.push(String::new());
            }
            updated.extend([
                TEXTORA_MANAGED_BEGIN.to_owned(),
                TEXTORA_MANAGED_RULE.to_owned(),
                TEXTORA_MANAGED_END.to_owned(),
            ]);
            Ok(updated)
        }
    }
}

pub fn repair_managed_ignore_block(rules: &[String]) -> Vec<String> {
    let mut user_rules = Vec::with_capacity(rules.len());
    let mut inside_managed_block = false;
    for rule in rules {
        if rule == TEXTORA_MANAGED_BEGIN {
            inside_managed_block = true;
            continue;
        }
        if inside_managed_block {
            if rule == TEXTORA_MANAGED_END {
                inside_managed_block = false;
            }
            continue;
        }
        if rule == TEXTORA_MANAGED_END || rule == TEXTORA_MANAGED_RULE {
            continue;
        }
        user_rules.push(rule.clone());
    }
    append_managed_ignore_block(&user_rules).expect("repair removes all managed block drift")
}

fn managed_block_positions(rules: &[String]) -> Vec<(usize, usize)> {
    let mut positions = Vec::new();
    let mut block_start = None;
    for (index, rule) in rules.iter().enumerate() {
        if rule == TEXTORA_MANAGED_BEGIN {
            if block_start.replace(index).is_some() {
                positions.push((index, index));
            }
        } else if rule == TEXTORA_MANAGED_END {
            if let Some(start) = block_start.take() {
                positions.push((start, index + 1));
            } else {
                positions.push((index, index + 1));
            }
        }
    }
    if let Some(start) = block_start {
        positions.push((start, rules.len()));
    }
    positions
}

#[cfg(test)]
mod tests {
    use super::{
        ManagedIgnoreState, TEXTORA_MANAGED_BEGIN, TEXTORA_MANAGED_END, TEXTORA_MANAGED_RULE,
        append_managed_ignore_block, inspect_managed_ignore_rules, repair_managed_ignore_block,
    };

    fn rules(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn appends_one_managed_block_without_touching_user_rules() {
        let existing = rules(&["# user rule", "*.bak", ""]);
        let updated = append_managed_ignore_block(&existing).expect("block should append");
        assert_eq!(
            updated,
            rules(&[
                "# user rule",
                "*.bak",
                "",
                TEXTORA_MANAGED_BEGIN,
                TEXTORA_MANAGED_RULE,
                TEXTORA_MANAGED_END,
            ])
        );
        assert_eq!(inspect_managed_ignore_rules(&updated), ManagedIgnoreState::Intact);
    }

    #[test]
    fn managed_block_insertion_is_idempotent() {
        let existing = rules(&["# user rule"]);
        let once = append_managed_ignore_block(&existing).expect("block should append");
        let twice = append_managed_ignore_block(&once).expect("existing block should remain");
        assert_eq!(once, twice);
    }

    #[test]
    fn detects_missing_and_drifted_managed_blocks() {
        assert_eq!(inspect_managed_ignore_rules(&rules(&["*.bak"])), ManagedIgnoreState::Missing);
        assert_eq!(
            inspect_managed_ignore_rules(&rules(&[
                TEXTORA_MANAGED_BEGIN,
                "wrong-rule",
                TEXTORA_MANAGED_END,
            ])),
            ManagedIgnoreState::Drifted
        );
        assert!(append_managed_ignore_block(&rules(&[TEXTORA_MANAGED_RULE])).is_err());
    }

    #[test]
    fn explicit_repair_replaces_drift_and_keeps_user_rules() {
        let drifted = rules(&[
            "# user rule",
            TEXTORA_MANAGED_BEGIN,
            "wrong-rule",
            TEXTORA_MANAGED_END,
            "*.bak",
        ]);
        let repaired = repair_managed_ignore_block(&drifted);
        assert_eq!(
            repaired,
            rules(&[
                "# user rule",
                "*.bak",
                "",
                TEXTORA_MANAGED_BEGIN,
                TEXTORA_MANAGED_RULE,
                TEXTORA_MANAGED_END,
            ])
        );
        assert_eq!(inspect_managed_ignore_rules(&repaired), ManagedIgnoreState::Intact);
    }
}
