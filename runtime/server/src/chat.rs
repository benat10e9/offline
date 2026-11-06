//! Conversations, messages and saved memories in SQLite on the user's computer
//! (ADR 0014). Nothing here is sent anywhere; the file lives in the OS app-data folder.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct ChatStore {
    conn: Mutex<Connection>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: i64,
    pub title: String,
    pub updated_at: i64,
    pub messages: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub role: String,
    pub content: String,
    /// For assistant messages: the verified answer (class, citations, sources) as JSON.
    pub meta: Option<serde_json::Value>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub text: String,
    pub created_at: i64,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Where conversations live by default: `KB_DATA_DIR`, else the OS app-data folder.
pub fn default_data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("KB_DATA_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        Path::new(&home).join("Library/Application Support/Offline Library")
    } else if cfg!(windows) {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| Path::new(&home).join("AppData/Roaming"))
            .join("Offline Library")
    } else {
        std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| Path::new(&home).join(".local/share"))
            .join("offline-library")
    }
}

impl ChatStore {
    pub fn open(dir: &Path) -> rusqlite::Result<Self> {
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join("conversations.sqlite");
        let conn = Connection::open(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS conversations (
               id INTEGER PRIMARY KEY, title TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS messages (
               id INTEGER PRIMARY KEY, conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
               role TEXT NOT NULL, content TEXT NOT NULL, meta TEXT, created_at INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS messages_conv ON messages(conversation_id, id);
             CREATE TABLE IF NOT EXISTS memories (
               id INTEGER PRIMARY KEY, text TEXT NOT NULL, created_at INTEGER NOT NULL);",
        )?;
        Ok(Self { conn: Mutex::new(conn), path })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
        let c = self.conn.lock().expect("chat store lock");
        f(&c)
    }

    pub fn conversations(&self) -> rusqlite::Result<Vec<ConversationSummary>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT c.id, c.title, c.updated_at, (SELECT COUNT(*) FROM messages m WHERE m.conversation_id = c.id)
                 FROM conversations c ORDER BY c.updated_at DESC, c.id DESC LIMIT 500",
            )?;
            let rows = st.query_map([], |r| {
                Ok(ConversationSummary { id: r.get(0)?, title: r.get(1)?, updated_at: r.get(2)?, messages: r.get(3)? })
            })?;
            rows.collect()
        })
    }

    pub fn create(&self) -> rusqlite::Result<i64> {
        self.with(|c| {
            let t = now();
            c.execute("INSERT INTO conversations (title, created_at, updated_at) VALUES ('', ?1, ?1)", params![t])?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn exists(&self, id: i64) -> bool {
        self.with(|c| c.query_row("SELECT 1 FROM conversations WHERE id = ?1", params![id], |_| Ok(())).optional())
            .map(|o| o.is_some())
            .unwrap_or(false)
    }

    pub fn delete(&self, id: i64) -> rusqlite::Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM conversations WHERE id = ?1", params![id])? > 0))
    }

    pub fn messages(&self, id: i64) -> rusqlite::Result<Vec<Message>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT id, role, content, meta, created_at FROM messages WHERE conversation_id = ?1 ORDER BY id",
            )?;
            let rows = st.query_map(params![id], |r| {
                let meta: Option<String> = r.get(3)?;
                Ok(Message {
                    id: r.get(0)?,
                    role: r.get(1)?,
                    content: r.get(2)?,
                    meta: meta.and_then(|m| serde_json::from_str(&m).ok()),
                    created_at: r.get(4)?,
                })
            })?;
            rows.collect()
        })
    }

    /// Append a message; the first user message becomes the conversation title.
    pub fn add_message(
        &self,
        id: i64,
        role: &str,
        content: &str,
        meta: Option<&serde_json::Value>,
    ) -> rusqlite::Result<i64> {
        self.with(|c| {
            let t = now();
            c.execute(
                "INSERT INTO messages (conversation_id, role, content, meta, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, role, content, meta.map(|m| m.to_string()), t],
            )?;
            let mid = c.last_insert_rowid();
            c.execute("UPDATE conversations SET updated_at = ?2 WHERE id = ?1", params![id, t])?;
            if role == "user" {
                let title: String = content.chars().take(60).collect();
                c.execute(
                    "UPDATE conversations SET title = ?2 WHERE id = ?1 AND title = ''",
                    params![id, title.trim()],
                )?;
            }
            Ok(mid)
        })
    }

    pub fn memories(&self) -> rusqlite::Result<Vec<Memory>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id, text, created_at FROM memories ORDER BY id")?;
            let rows = st.query_map([], |r| Ok(Memory { id: r.get(0)?, text: r.get(1)?, created_at: r.get(2)? }))?;
            rows.collect()
        })
    }

    pub fn add_memory(&self, text: &str) -> rusqlite::Result<i64> {
        self.with(|c| {
            c.execute("INSERT INTO memories (text, created_at) VALUES (?1, ?2)", params![text.trim(), now()])?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn delete_memory(&self, id: i64) -> rusqlite::Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM memories WHERE id = ?1", params![id])? > 0))
    }
}

/// Is this a follow-up that only makes sense with the previous question
/// ("What if I don't have bleach?", "What about for kids?")?
pub fn is_follow_up(question: &str) -> bool {
    let q = question.trim().to_lowercase();
    let words: Vec<&str> = q.split(|c: char| !c.is_alphanumeric() && c != '\'').filter(|w| !w.is_empty()).collect();
    let starts = [
        "what about",
        "what if",
        "how about",
        "and ",
        "but ",
        "also",
        "then ",
        "so ",
        "why",
        "is that",
        "is it",
        "does it",
        "can it",
        "will it",
        "how long",
        "how much",
        "how often",
    ];
    let pronoun = words.iter().any(|w| {
        matches!(
            *w,
            "it" | "that" | "this" | "they" | "them" | "those" | "these" | "there" | "he" | "she" | "him" | "her"
        )
    });
    words.len() <= 4 || starts.iter().any(|s| q.starts_with(s)) || (pronoun && words.len() <= 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_roundtrip_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let s = ChatStore::open(dir.path()).unwrap();
            let id = s.create().unwrap();
            s.add_message(id, "user", "How do I purify river water?", None).unwrap();
            s.add_message(
                id,
                "assistant",
                "Boil it for one minute [S1].",
                Some(&serde_json::json!({"class": "verified"})),
            )
            .unwrap();
            s.add_memory("We have a Generac GP6500 generator").unwrap();
            id
        };
        // Reopen: survives an app restart.
        let s = ChatStore::open(dir.path()).unwrap();
        let convs = s.conversations().unwrap();
        assert_eq!(convs[0].title, "How do I purify river water?");
        assert_eq!(convs[0].messages, 2);
        let m = s.messages(id).unwrap();
        assert_eq!(m[1].meta.as_ref().unwrap()["class"], "verified");
        assert_eq!(s.memories().unwrap()[0].text, "We have a Generac GP6500 generator");
        assert!(s.delete(id).unwrap());
        assert!(s.messages(id).unwrap().is_empty(), "messages deleted with the conversation");
        assert!(!s.exists(id));
    }

    #[test]
    fn follow_ups() {
        assert!(is_follow_up("What if I don't have bleach?"));
        assert!(is_follow_up("What about for my kids?"));
        assert!(is_follow_up("How long does it last?"));
        assert!(!is_follow_up("How do I purify river water when camping?"));
        assert!(!is_follow_up("What are the symptoms of carbon monoxide poisoning?"));
    }
}
