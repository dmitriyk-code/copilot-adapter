# Feature Name — Design/Plan Document Template

**Status:** [Draft | Proposed | In Progress | Implemented | Complete | Deprecated]
**Date:** YYYY-MM-DD
**Severity:** [Low | Medium | High | Critical] (for bug fixes/issues)
**Related:** [Links to related design docs, issue trackers, etc.]
**Based on:** [For plan documents - link to design document]
**Prerequisite:** [Dependencies that must be complete first]

---

## Executive Summary

[1-3 paragraph overview of the problem and proposed solution]

Key points:
- What is being built/fixed
- Why it matters
- High-level approach
- Expected outcome

---

## Context / Background

### Current State

[Describe the current implementation or situation]
- What exists today
- How it currently works
- What's wrong or missing

### Target State / Desired Behavior

[Describe what the end state should look like]
- What the implementation should do
- How it should work
- What problems it solves

---

## Problem Statement

[Clear, concise statement of the problem being solved]

**Observed behavior:**
- What users/developers experience today

**Expected behavior:**
- What should happen instead

**Impact:**
- Who is affected
- Severity of the issue

---

## Goals and Non-Goals

### Goals

| ID | Goal | Success Criteria |
|----|------|------------------|
| G1 | [Specific, measurable goal] | [How to verify success] |
| G2 | ... | ... |

### Non-Goals

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | [What we're explicitly NOT doing] | [Why not] |
| NG2 | ... | ... |

---

## Research / Analysis

[For design documents - research findings, experiments, API investigation]

### Key Findings

[Important discoveries that inform the design]

### Options Considered

#### Option A: [Name] (Recommended/Not Recommended)

**Description:** [How it works]

**Pros:**
- [Advantage 1]
- [Advantage 2]

**Cons:**
- [Disadvantage 1]
- [Disadvantage 2]

#### Option B: [Name]

[Similar structure]

### Recommended Approach

[Which option and why]

---

## Proposed Design / Architecture

[For design documents]

### Component Overview

```
[ASCII diagram showing architecture]
```

### Technical Details

[Detailed technical specifications]

### API/Interface Design

[If applicable - endpoint specs, function signatures, etc.]

---

## Implementation Plan

[For plan documents - break down into epics and tasks]

### Epic 1: [Epic Name] (Day X, Y days)

**Status:** [Not Started | In Progress | Done]

#### Task 1.1: [Task Name]

**File:** `path/to/file.rs` (NEW/MODIFIED)

**Description:** [What needs to be done]

**Code:**
```rust
// Example implementation
```

**Acceptance Criteria:**
- [ ] Criterion 1
- [ ] Criterion 2

#### Task 1.2: [Another Task]

[Similar structure]

### Epic 2: [Next Epic]

[Similar structure]

---

## Requirements

### Functional Requirements

| ID | Requirement | Source |
|----|-------------|--------|
| FR1 | [Specific functional requirement] | [Where it came from] |
| FR2 | ... | ... |

### Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | [Performance, security, etc.] | [Specific target/threshold] |
| NFR2 | ... | ... |

---

## File Changes Summary

| File | Change | Description |
|------|--------|-------------|
| `src/path/file.rs` | Modified | [What changed] |
| `src/new/file.rs` | **New file** | [What it does] |
| `src/old/file.rs` | Deleted | [Why removed] |

---

## Testing Strategy

### Unit Tests

1. **Test category 1:**
   - Test case 1
   - Test case 2

2. **Test category 2:**
   - Test case 1

### Integration Tests

1. **Scenario 1:**
   - Setup
   - Action
   - Verification

### Manual E2E Tests

1. **Test 1:**
   - Steps
   - Expected result

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| [Specific risk] | High/Medium/Low | High/Medium/Low | [How to mitigate] |
| ... | ... | ... | ... |

---

## Success Criteria

1. **Criterion 1** — [How to measure]
2. **Criterion 2** — [How to measure]
3. **Criterion 3** — [How to measure]

---

## Performance / Metrics

[If applicable - performance targets, benchmarks, metrics to track]

| Metric | Target | How to Measure |
|--------|--------|----------------|
| Latency | < 10ms | [Measurement method] |
| ... | ... | ... |

---

## Rollout / Migration Plan

[If applicable - how to deploy or migrate]

### Phase 1: [Phase Name]

- [ ] Step 1
- [ ] Step 2

### Phase 2: [Phase Name]

[Similar structure]

---

## Design Decisions

[Key decisions made and their rationale]

| Decision | Rationale |
|----------|-----------|
| [Specific decision] | [Why this choice] |
| ... | ... |

---

## Open Questions

| # | Question | Status |
|---|----------|--------|
| 1 | [Unanswered question] | Open/Resolved/Deferred |
| 2 | ... | ... |

---

## Documentation Updates

### Files to Update

| File | Changes |
|------|---------|
| `README.md` | [What to update] |
| `CLAUDE.md` | [What to update] |
| ... | ... |

---

## Verification Steps

[How to verify the implementation works]

1. **Step 1:** [Action]
   - Expected result

2. **Step 2:** [Action]
   - Expected result

---

## References

- [Link to related documentation]
- [Link to external resources]
- [Link to code examples]
- [Link to API docs]

---

## Appendix

[Optional additional information, detailed code examples, etc.]

---

## Common Patterns Found in copilot-adapter Design Documents

### Design Document Pattern

Design documents typically include:
1. **Research section** — Investigation of existing solutions (e.g., LiteLLM), API behavior, endpoint discovery
2. **Options comparison** — Multiple approaches with pros/cons
3. **Technical details** — Type definitions, API formats, data structures
4. **Verification scripts** — Bash/PowerShell scripts to test the design assumptions

### Plan Document Pattern

Plan documents typically include:
1. **Epic-based breakdown** — Features split into logical epics with time estimates
2. **Task granularity** — Each epic broken into specific, actionable tasks
3. **File-level changes** — Exactly which files to create/modify/delete
4. **Code examples** — Concrete implementation snippets for each task
5. **Acceptance criteria** — Checkboxes for each task to track completion

### Relationship Pattern

- Design documents are exploratory and research-focused
- Plan documents reference their corresponding design document via `**Based on:**` header
- Both use consistent status tracking and metadata headers
- Both include comprehensive testing strategies

### Common Sections Across Both

- Executive Summary (always first after metadata)
- Background/Context (understanding current state)
- Goals and Non-Goals (clear scope)
- File Changes Summary (impact assessment)
- Testing Strategy (quality assurance)
- Success Criteria (definition of done)
- References (external links and dependencies)
