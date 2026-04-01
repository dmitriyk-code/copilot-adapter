# Feature Name — Implementation Plan

**Status:** [Not Started | In Progress | Done]
**Date:** YYYY-MM-DD
**Based on:** [Link to design document, e.g., FEATURE-NAME.design.md]
**Prerequisite:** [Dependencies that must be complete first]
**Estimated Time:** X-Y days

---

## Executive Summary

[1-2 paragraph overview of what will be implemented]

This plan implements:
- Key feature 1
- Key feature 2
- Key feature 3

**Total estimated time:** X-Y days

---

## Background

### Current State

[Brief description of current implementation]
- What exists today
- Relevant file locations
- Current behavior

### Target State

[Brief description of desired end state]
- What will exist after implementation
- Key changes
- Expected behavior

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

## Implementation Plan

### Epic 1: [Epic Name] (Day X, Y days)

**Status:** [Not Started | In Progress | Done]

**Objective:** [What this epic accomplishes]

#### Task 1.1: [Task Name]

**File:** `path/to/file.rs` (NEW/MODIFIED/DELETED)

**Description:** [What needs to be done]

**Implementation:**
```rust
// Example code snippet showing the change
pub struct NewType {
    field: String,
}
```

**Acceptance Criteria:**
- [ ] Criterion 1
- [ ] Criterion 2
- [ ] Unit tests passing

**Notes:** [Any additional context or gotchas]

#### Task 1.2: [Task Name]

**File:** `path/to/another.rs` (MODIFIED)

**Description:** [What needs to be done]

**Implementation:**
```rust
// Before
fn old_implementation() {}

// After
fn new_implementation() {}
```

**Acceptance Criteria:**
- [ ] Criterion 1
- [ ] Criterion 2

---

### Epic 2: [Epic Name] (Day X, Y days)

**Status:** [Not Started | In Progress | Done]

**Objective:** [What this epic accomplishes]

#### Task 2.1: [Task Name]

[Similar structure to Epic 1 tasks]

---

### Epic 3: Testing (Day X, Y days)

**Status:** [Not Started | In Progress | Done]

**Objective:** Ensure implementation is thoroughly tested

#### Task 3.1: Unit Tests

**File:** `tests/unit/feature_tests.rs` (NEW)

**Tests to implement:**
1. **Test category 1:**
   ```rust
   #[test]
   fn test_specific_behavior() {
       // Test implementation
   }
   ```
   - [ ] Test passes
   
2. **Test category 2:**
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All unit tests passing
- [ ] Code coverage > X%

#### Task 3.2: Integration Tests

**File:** `tests/integration/feature_tests.rs` (NEW)

**Scenarios to test:**
1. **Scenario 1:** [Description]
   - Setup: [What to configure]
   - Action: [What to do]
   - Verification: [What to check]
   - [ ] Test passes

2. **Scenario 2:** [Description]
   - [ ] Test passes

**Acceptance Criteria:**
- [ ] All integration tests passing
- [ ] End-to-end flow verified

#### Task 3.3: Manual E2E Tests

**File:** `docs/e2e-testing.md` (MODIFIED)

**Test procedures to add:**
1. **Test 1:** [Description]
   ```bash
   # Steps
   copilot-adapter start
   curl ...
   ```
   - Expected result: [What should happen]
   - [ ] Documented

**Acceptance Criteria:**
- [ ] E2E test procedures documented
- [ ] Manual tests executed and verified

---

### Epic 4: Documentation (Day X, Y days)

**Status:** [Not Started | In Progress | Done]

**Objective:** Update all relevant documentation

#### Task 4.1: Update README

**File:** `README.md` (MODIFIED)

**Changes:**
- Add new feature to features list
- Update usage examples
- Add configuration options

**Acceptance Criteria:**
- [ ] README updated
- [ ] Examples tested

#### Task 4.2: Update CLAUDE.md

**File:** `CLAUDE.md` (MODIFIED)

**Changes:**
- Add feature notes
- Update architecture section
- Add troubleshooting tips

**Acceptance Criteria:**
- [ ] CLAUDE.md updated

#### Task 4.3: Update API Documentation

**File:** `docs/api.md` (MODIFIED/NEW)

**Changes:**
- Document new endpoints/behavior
- Add request/response examples
- Update limitations section

**Acceptance Criteria:**
- [ ] API docs updated
- [ ] Examples accurate

---

## Requirements

### Functional Requirements

| ID | Requirement | Source | Epic |
|----|-------------|--------|------|
| FR1 | [Specific functional requirement] | [Design doc section] | Epic 1 |
| FR2 | ... | ... | Epic 2 |

### Non-Functional Requirements

| ID | Requirement | Target | Epic |
|----|-------------|--------|------|
| NFR1 | [Performance, security, etc.] | [Specific target] | Epic 1 |
| NFR2 | ... | ... | ... |

---

## File Changes Summary

| File | Change | Epic | Description |
|------|--------|------|-------------|
| `src/path/file.rs` | Modified | Epic 1 | [What changed] |
| `src/new/file.rs` | **New file** | Epic 1 | [What it does] |
| `src/old/file.rs` | Deleted | Epic 2 | [Why removed] |
| `tests/unit/tests.rs` | **New file** | Epic 3 | Unit tests |
| `README.md` | Modified | Epic 4 | Documentation |

---

## Testing Strategy

### Test Coverage

| Component | Unit Tests | Integration Tests | E2E Tests |
|-----------|------------|-------------------|-----------|
| Component 1 | Epic 3.1 | Epic 3.2 | Epic 3.3 |
| Component 2 | Epic 3.1 | Epic 3.2 | - |

### Test Files

| File | Type | Coverage |
|------|------|----------|
| `tests/unit/feature_tests.rs` | Unit | New functionality |
| `tests/integration/feature_tests.rs` | Integration | End-to-end flows |
| `docs/e2e-testing.md` | Manual E2E | User workflows |

---

## Dependencies

### External Dependencies

| Dependency | Version | Purpose | Epic |
|------------|---------|---------|------|
| `crate-name` | 1.2.3 | [What it's for] | Epic 1 |

**Cargo.toml changes:**
```toml
[dependencies]
crate-name = "1.2.3"
```

### Internal Dependencies

| Module | Required By | Status |
|--------|-------------|--------|
| `module_a` | Epic 1 | ✅ Exists |
| `module_b` | Epic 2 | 🚧 Will create |

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation | Epic |
|------|--------|-------------|------------|------|
| [Specific risk] | High/Medium/Low | High/Medium/Low | [How to mitigate] | Epic 1 |
| ... | ... | ... | ... | ... |

---

## Success Criteria

1. **Criterion 1** — [How to measure] (Epic 1)
2. **Criterion 2** — [How to measure] (Epic 2)
3. **All tests passing** — Unit, integration, and E2E tests pass (Epic 3)
4. **Documentation complete** — All docs updated (Epic 4)
5. **Performance targets met** — [Specific metrics] (Epic X)

---

## Rollout / Migration Plan

### Phase 1: Development (Epics 1-2)
- [ ] Implement core functionality
- [ ] Initial testing
- [ ] Code review

### Phase 2: Testing (Epic 3)
- [ ] Unit tests complete
- [ ] Integration tests complete
- [ ] Manual E2E verification

### Phase 3: Documentation (Epic 4)
- [ ] README updated
- [ ] CLAUDE.md updated
- [ ] API docs updated

### Phase 4: Release
- [ ] All acceptance criteria met
- [ ] Final review
- [ ] Merge to main
- [ ] Archive design/plan docs

---

## Epic Status Tracking

| Epic | Status | Start Date | End Date | Notes |
|------|--------|------------|----------|-------|
| Epic 1 | Not Started | - | - | |
| Epic 2 | Not Started | - | - | |
| Epic 3 | Not Started | - | - | |
| Epic 4 | Not Started | - | - | |

---

## Open Questions

| # | Question | Status | Blocker For |
|---|----------|--------|-------------|
| 1 | [Unanswered question] | Open/Resolved/Deferred | Epic X |
| 2 | ... | ... | ... |

---

## References

- [Design document](./FEATURE-NAME.design.md)
- [Related plan document](./RELATED.plan.md)
- [External documentation]()
- [API reference]()

---

## Notes

### Development Notes
- [Notes added during implementation]
- [Lessons learned]
- [Gotchas discovered]

### Review Notes
- [Code review feedback]
- [Design review comments]

### Testing Notes
- [Test failures and fixes]
- [Performance observations]
- [Edge cases discovered]
