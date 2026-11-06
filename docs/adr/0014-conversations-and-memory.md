# ADR 0014: Conversations and memory live on the user's computer, in SQLite

Status: Accepted (2026-11-06). Amends ADR 0007 for conversation data.

## Decision

The Ask view is a conversation: history in a sidebar, streamed answers, follow-up questions. Conversations,
messages and saved notes ("memories") are stored in SQLite in the OS app-data folder
(macOS `~/Library/Application Support/Offline Library`, Windows `%APPDATA%\Offline Library`,
Linux `~/.local/share/offline-library`; `KB_DATA_DIR` overrides). Not on the USB drive by default.

Memory is explicit in v1: the user saves notes ("Remember" on a message, or the Library tab), and the signed drive
profile's equipment is included. The model does not extract memories on its own yet: a small model writing wrong
"facts" into memory would silently mislead every later answer. Model-suggested memories, approved by the user,
come with larger-model testing.

Notes and profile equipment go into the prompt labelled as the user's own statements, never as evidence; they
cannot be cited and cannot make an answer Verified. Follow-ups ("what if I don't have bleach?") are searched
together with the previous question, and judged for relevance against that earlier question (the topic).

## Why

Privacy and durability: conversations survive drive removal, and a lost drive exposes no conversation history.
An encrypted portable mode (history on the drive) is a later option for customers who want it.
