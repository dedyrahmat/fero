use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Accent {
    #[default]
    Harbor,
    Ember,
    Sage,
    Indigo,
}

#[derive(Clone, Default)]
pub struct Preferences {
    pub theme: Theme,
    pub accent: Accent,
    pub compact: bool,
    pub reduce_motion: bool,
    pub collapsed: bool,
    pub default_folder: Option<PathBuf>,
    pub start_immediately: bool,
    pub start_paused: bool,
}

impl Preferences {
    pub fn download_folder(&self) -> Option<PathBuf> {
        self.default_folder
            .clone()
            .or_else(dirs::download_dir)
            .or_else(|| dirs::home_dir().map(|path| path.join("Downloads")))
    }

    pub fn open(path: &Path) -> Result<(Connection, Self)> {
        let mut db = Connection::open(path)?;
        let schema = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema.execute_batch(
            "CREATE TABLE IF NOT EXISTS preferences (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                theme INTEGER NOT NULL,
                accent INTEGER NOT NULL,
                compact INTEGER NOT NULL,
                collapsed INTEGER NOT NULL,
                default_folder TEXT,
                start_immediately INTEGER NOT NULL,
                start_paused INTEGER NOT NULL,
                reduce_motion INTEGER NOT NULL DEFAULT 0
            )",
        )?;
        let has_motion_column = schema.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('preferences') WHERE name = 'reduce_motion')",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !has_motion_column {
            schema.execute_batch(
                "ALTER TABLE preferences ADD COLUMN reduce_motion INTEGER NOT NULL DEFAULT 0",
            )?;
        }
        schema.commit()?;
        let preferences = db
            .query_row(
                "SELECT theme, accent, compact, collapsed, default_folder, start_immediately,
                        start_paused, reduce_motion FROM preferences WHERE id = 1",
                [],
                |row| {
                    Ok(Self {
                        theme: match row.get(0)? {
                            1_i64 => Theme::Light,
                            2 => Theme::Dark,
                            _ => Theme::System,
                        },
                        accent: match row.get(1)? {
                            1_i64 => Accent::Sage,
                            2 => Accent::Indigo,
                            3 => Accent::Harbor,
                            _ => Accent::Ember,
                        },
                        compact: row.get(2)?,
                        collapsed: row.get(3)?,
                        default_folder: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
                        start_immediately: row.get(5)?,
                        start_paused: row.get(6)?,
                        reduce_motion: row.get(7)?,
                    })
                },
            )
            .optional()?;
        let preferences = match preferences {
            Some(saved) => saved,
            None => {
                let defaults = Self::default();
                defaults.save(&db)?;
                defaults
            }
        };
        Ok((db, preferences))
    }

    pub fn save(&self, db: &Connection) -> Result<()> {
        let theme = match self.theme {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        };
        let accent = match self.accent {
            Accent::Ember => 0,
            Accent::Sage => 1,
            Accent::Indigo => 2,
            Accent::Harbor => 3,
        };
        db.execute(
            "INSERT INTO preferences (id, theme, accent, compact, collapsed, start_immediately, start_paused, reduce_motion)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET theme = excluded.theme, accent = excluded.accent,
                 compact = excluded.compact, collapsed = excluded.collapsed,
                 start_immediately = excluded.start_immediately, start_paused = excluded.start_paused,
                 reduce_motion = excluded.reduce_motion",
            params![
                theme,
                accent,
                self.compact,
                self.collapsed,
                self.start_immediately,
                self.start_paused,
                self.reduce_motion,
            ],
        )?;
        Ok(())
    }

    pub fn save_folder(db: &Connection, folder: Option<&Path>) -> Result<()> {
        let folder = folder
            .map(|path| {
                ensure!(
                    path.is_absolute() && path.is_dir(),
                    "Choose an existing download folder"
                );
                path.to_str().context("Download folder must be valid UTF-8")
            })
            .transpose()?;
        db.execute(
            "UPDATE preferences SET default_folder = ?1 WHERE id = 1",
            [folder],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_keep_appearance_and_download_defaults_across_relaunch() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("library.sqlite3");
        let (db, defaults) = Preferences::open(&path)?;
        assert!(defaults.theme == Theme::System && defaults.accent == Accent::Harbor);
        assert!(Preferences::open(&path)?.1.accent == Accent::Harbor);
        db.execute_batch(
            "UPDATE preferences SET accent = 0 WHERE id = 1;
                          ALTER TABLE preferences DROP COLUMN reduce_motion;",
        )?;
        let (_, migrated) = Preferences::open(&path)?;
        assert!(migrated.accent == Accent::Ember && !migrated.reduce_motion);
        assert!(!defaults.start_immediately);
        let settings = Preferences {
            theme: Theme::Dark,
            accent: Accent::Indigo,
            compact: true,
            reduce_motion: true,
            collapsed: true,
            start_immediately: true,
            start_paused: true,
            ..Preferences::default()
        };
        Preferences::save_folder(&db, Some(dir.path()))?;
        settings.save(&db)?;
        assert!(Preferences::save_folder(&db, Some(&dir.path().join("missing"))).is_err());
        drop(db);
        let (_, saved) = Preferences::open(&path)?;
        assert!(saved.theme == Theme::Dark && saved.accent == Accent::Indigo);
        assert!(saved.compact && saved.collapsed && saved.start_immediately && saved.start_paused);
        assert!(saved.reduce_motion);
        assert_eq!(saved.download_folder().as_deref(), Some(dir.path()));
        Ok(())
    }
}
