# ADR 0003: Lexical-first retrieval over prebuilt Tantivy indexes

Status: Accepted (2026-11-05)

## Decision

Each package ships a prebuilt BM25 index over passages with stable IDs. Indexes are opened read-only. Customer machines never build public indexes or embeddings. Dense vectors are optional and only for small curated packs.

## Why

Scales to terabytes on consumer hardware; deterministic; no embedding compute at the customer.
