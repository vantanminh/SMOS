# Feature Intake

Every implementation prompt enters the intake gate before code changes.

## Lanes

### Tiny

Low-risk docs, copy, or narrow edits. Record intent, patch directly, run quick
checks. No full story packet required.

### Normal

Story-sized behavior with bounded blast radius. Create a story from
`docs/templates/story.md`, link product docs, define validation, implement the
smallest vertical slice.

### High-Risk

Touches security, data model, multi-role contracts, or large scope. Use
`docs/templates/` high-risk structure when available, require explicit proof,
and record durable decisions for locked choices.

## Checklist (escalate if any apply)

- Auth / sessions
- Authorization / tenancy
- Data model / migrations
- Audit / sensitive data
- External providers
- Public API contracts
