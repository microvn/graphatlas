//! `ga init` — write `.claude/skills/graphatlas.md` into the project.
//!
//! Idempotent: if the file already exists, replace it (skills are
//! ship-versioned with the binary, so the latest content always wins).

use super::json_io::{atomic_write_bytes, ensure_parent_dir};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Skill markdown content, baked into the binary at compile time.
pub const SKILL_MD: &str = include_str!("../../assets/graphatlas-skill.md");

/// Relative path within the project where the skill is written.
pub const SKILL_REL_PATH: &str = ".claude/skills/graphatlas.md";

#[derive(Debug, PartialEq, Eq)]
pub enum SkillOutcome {
    Created(PathBuf),
    Updated(PathBuf),
    Unchanged(PathBuf),
}

pub fn install_skill(project_root: &Path) -> Result<SkillOutcome> {
    let target = project_root.join(SKILL_REL_PATH);
    ensure_parent_dir(&target)?;
    let existed = target.exists();
    if existed {
        let current = std::fs::read(&target).unwrap_or_default();
        if current == SKILL_MD.as_bytes() {
            return Ok(SkillOutcome::Unchanged(target));
        }
    }
    atomic_write_bytes(&target, SKILL_MD.as_bytes())?;
    Ok(if existed {
        SkillOutcome::Updated(target)
    } else {
        SkillOutcome::Created(target)
    })
}

pub fn uninstall_skill(project_root: &Path) -> Result<bool> {
    let target = project_root.join(SKILL_REL_PATH);
    if target.exists() {
        std::fs::remove_file(&target)?;
        Ok(true)
    } else {
        Ok(false)
    }
}
