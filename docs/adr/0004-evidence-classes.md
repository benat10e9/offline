# ADR 0004: Mandatory evidence classes decided in code

Status: Accepted (2026-11-05)

## Decision

Every answer is labelled verified, partially supported or unsupported by `kb-evidence`, from manifest facts: document identity, equipment applicability, revision/supersession, jurisdiction and authority. The model cannot raise the class. Unsupported questions are not sent to the model. Answers whose citations do not resolve to passages shown that turn are withheld.

## Why

Model-generated confidence is not reliable enough for medical, police or military use. See spec/evidence-classes.md.
