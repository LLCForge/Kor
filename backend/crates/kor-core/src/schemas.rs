//! Versioned JSON Schemas of structured model output (spec §15.13). Files live in
//! `kor-core/schemas/`; a new version gets a new file, old ones stay for stored data.

/// Orchestrator plan for `kor task` (ADR 0037).
pub const TASK_PLAN_V1: &str = include_str!("../schemas/task-plan.v1.json");
/// Reviewer verdict on one sub-agent result.
pub const TASK_REVIEW_V1: &str = include_str!("../schemas/task-review.v1.json");
/// Orchestrator re-plan of one conflicting sub-task.
pub const TASK_REPLAN_V1: &str = include_str!("../schemas/task-replan.v1.json");

#[cfg(test)]
mod tests {
    #[test]
    fn schemas_are_json_objects() {
        for s in [super::TASK_PLAN_V1, super::TASK_REVIEW_V1, super::TASK_REPLAN_V1] {
            let v: serde_json::Value = serde_json::from_str(s).unwrap();
            assert_eq!(v["type"], "object");
        }
    }
}
