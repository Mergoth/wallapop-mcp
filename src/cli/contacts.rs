use crate::{
    Error, Result,
    client::{atomic_save, read_file},
    identifier,
};
use serde_json::Value;
use std::{collections::BTreeSet, path::PathBuf};

const LIMIT: usize = 10_000;

pub(super) struct Contacts {
    path: Option<PathBuf>,
    ids: BTreeSet<String>,
}
impl Contacts {
    pub(super) fn load(path: Option<&str>) -> Result<Self> {
        let path = path.map(PathBuf::from);
        let mut ids = BTreeSet::new();
        if let Some(p) = path.as_ref().filter(|p| p.exists()) {
            let data: Value = serde_json::from_slice(&read_file(p, 2 << 20)?)
                .map_err(|_| Error::new("invalid_contact_history"))?;
            for id in data
                .as_array()
                .filter(|a| a.len() <= LIMIT)
                .ok_or(Error::new("invalid_contact_history"))?
            {
                ids.insert(identifier(id.as_str().unwrap_or(""))?.to_string());
            }
        }
        Ok(Self { path, ids })
    }

    pub(super) fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    // Record attempts before sending, so an uncertain outcome stays deduplicated
    // after a restart. A failed persistence step must prevent the network write.
    pub(super) fn record(&mut self, id: &str) -> Result<()> {
        identifier(id)?;
        if self.ids.len() >= LIMIT && !self.contains(id) {
            return Err(Error::new("contact_history_limit"));
        }
        self.ids.insert(id.into());
        if let Some(path) = &self.path {
            atomic_save(path, &self.ids.iter().cloned().map(Value::String).collect())
                .map_err(|_| Error::new("contact_history_persistence_failed"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_history_and_new_attempts_survive_reload() {
        let dir = std::env::temp_dir().join(format!("contacts-{}", crate::uuid().unwrap()));
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("contacted.json");
        std::fs::write(&file, br#"["previous"]"#).unwrap();
        let mut contacts = Contacts::load(file.to_str()).unwrap();
        assert!(contacts.contains("previous"));
        contacts.record("new-attempt").unwrap();
        let reloaded = Contacts::load(file.to_str()).unwrap();
        assert!(reloaded.contains("previous") && reloaded.contains("new-attempt"));
        std::fs::write(&file, b"{}").unwrap();
        assert!(Contacts::load(file.to_str()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn history_is_bounded_and_persistence_failure_is_reported() {
        let mut contacts = Contacts {
            path: None,
            ids: (0..LIMIT).map(|i| format!("item-{i}")).collect(),
        };
        assert_eq!(
            contacts.record("one-too-many").unwrap_err().code,
            "contact_history_limit"
        );
        contacts.record("item-0").unwrap();
        contacts.ids.clear();
        let dir = std::env::temp_dir().join(format!("contacts-{}", crate::uuid().unwrap()));
        std::fs::create_dir(&dir).unwrap();
        contacts.path = Some(dir.clone());
        assert_eq!(
            contacts.record("item").unwrap_err().code,
            "contact_history_persistence_failed"
        );
        std::fs::remove_dir(dir).unwrap();
    }
}
