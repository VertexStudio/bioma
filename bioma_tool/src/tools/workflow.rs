//! # Workflow Tool
//!
//! The Workflow Tool helps manage complex multi-step processes, allowing for sequential progression,
//! branching paths, and step revisions.
//!
//! ## Core Concepts
//!
//! ### Steps
//! A workflow consists of steps, each with:
//! - **step_number**: Position in the sequence
//! - **total_steps**: Estimated total number of steps
//! - **step_description**: Description of the current step
//! - **next_step_needed**: Boolean indicating if more steps are required
//!
//! ### Branching
//! Create alternative paths from any existing step:
//! - **branch_id**: Unique identifier for the branch
//! - **branch_from_step**: Step number from which to branch
//!
//! ### Revisions
//! Modify previous steps when needed:
//! - **is_step_revision**: Flag indicating this is a revision
//! - **revises_step**: Step number being revised
//!
//! ## Example Usage
//!
//! ### Basic Sequential Flow
//! ```json
//! Step 1: {
//!   "step_description": "Gather requirements",
//!   "step_number": 1,
//!   "total_steps": 3,
//!   "next_step_needed": true
//! }
//!
//! Step 2: {
//!   "step_description": "Design solution",
//!   "step_number": 2,
//!   "total_steps": 3,
//!   "next_step_needed": true
//! }
//!
//! Step 3: {
//!   "step_description": "Implement solution",
//!   "step_number": 3,
//!   "total_steps": 3,
//!   "next_step_needed": false
//! }
//! ```
//!
//! ### Creating a Branch
//! ```json
//! Step 3a: {
//!   "step_description": "Alternative implementation",
//!   "step_number": 3,
//!   "total_steps": 3,
//!   "next_step_needed": true,
//!   "branch_id": "alternative",
//!   "branch_from_step": 2
//! }
//! ```
//!
//! ### Revising a Step
//! ```json
//! Step 2 (revised): {
//!   "step_description": "Redesign solution based on feedback",
//!   "step_number": 2,
//!   "total_steps": 3,
//!   "next_step_needed": true,
//!   "is_step_revision": true,
//!   "revises_step": 2
//! }
//! ```

use crate::schema::{CallToolResult, TextContent};
use crate::tools::{ToolDef, ToolError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Represents a single step in a workflow process
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "A single step in the workflow process")]
pub struct WorkflowStep {
    #[schemars(description = "Detailed description of what this step accomplishes", required = true)]
    step_description: String,

    #[schemars(description = "Current position in the workflow sequence (e.g., 1 for first step)", required = true)]
    step_number: i32,

    #[schemars(description = "Estimated total number of steps in the complete workflow", required = true)]
    total_steps: i32,

    #[schemars(
        description = "Set to true if another step will follow this one, false if this is the final step",
        required = true
    )]
    next_step_needed: bool,

    #[schemars(description = "Set to true if this step revises a previous step")]
    is_step_revision: Option<bool>,

    #[schemars(description = "If revising a previous step, specify which step number is being revised")]
    revises_step: Option<i32>,

    #[schemars(description = "If creating a branch, specify which step number this branch starts from")]
    branch_from_step: Option<i32>,

    #[schemars(description = "A unique identifier for this branch (required when creating a branch)")]
    branch_id: Option<String>,

    #[schemars(description = "Indicates whether additional steps are required to complete the workflow")]
    needs_more_steps: Option<bool>,
}

/// Query parameters for retrieving workflow steps
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Query parameters to retrieve specific workflow steps")]
pub struct StepQuery {
    #[schemars(description = "Retrieve a specific step by its number")]
    step_number: Option<i32>,

    #[schemars(description = "Retrieve all steps from a specific branch")]
    branch_id: Option<String>,

    #[schemars(description = "Return only the most recent N steps")]
    return_last_n: Option<i32>,
}

/// Enhanced response providing detailed workflow status
#[derive(Debug, Serialize, Deserialize)]
struct WorkflowStatus {
    step_number: i32,
    total_steps: i32,
    next_step_needed: bool,
    last_step_description: String,
    current_branch: Option<String>,
    branches: Vec<String>,
    step_history_length: usize,
    recent_steps: Vec<WorkflowStep>, // Last few steps for context
    active_branches: HashMap<String, BranchStatus>,
    progress_visualization: String,
}

/// Status information for a workflow branch
#[derive(Debug, Serialize, Deserialize)]
struct BranchStatus {
    branch_name: String,
    created_at_step: i32,
    steps_in_branch: i32,
    is_active: bool,
}

/// Internal state tracking for the workflow
#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkflowState {
    step_history: Vec<WorkflowStep>,
    branches: HashMap<String, Vec<WorkflowStep>>,
    current_branch: Option<String>,
}

/// Workflow tool for managing multi-step processes
#[derive(Clone, Serialize, Deserialize)]
pub struct Workflow {
    #[serde(skip)]
    state: Arc<Mutex<WorkflowState>>,
    allow_branches: bool,
    max_steps: Option<i32>,
}

impl Default for Workflow {
    fn default() -> Self {
        Self { state: Arc::new(Mutex::new(WorkflowState::default())), allow_branches: true, max_steps: None }
    }
}

impl ToolDef for Workflow {
    const NAME: &'static str = "mcp_bioma_workflow";

    const DESCRIPTION: &'static str = r#"# Advanced Workflow Tool

This tool helps you manage dynamic problem-solving through a flexible, adaptable process that evolves as your understanding deepens. Use it to structure complex multi-step workflows while allowing for reflection, course correction, and exploratory thinking paths.

## When You Should Use This Tool

- When breaking down complex problems requiring multiple steps
- For planning processes that may need revision as they develop
- During analysis requiring course correction or branching exploration
- When tackling problems where the full scope isn't initially clear
- For tasks needing persistent context across steps
- To filter irrelevant information while maintaining focus

## Core Concepts

### Steps
Structure your workflow with adaptable steps, each containing:
- **step_number**: Your current position in the sequence
- **total_steps**: Your estimated total steps (adjust as you progress)
- **step_description**: Detailed description of your current thinking
- **next_step_needed**: Set to true if you need more steps
- **needs_more_steps**: Set to true to continue even after reaching what seemed like the "final" step

### Branching
Create alternative paths when you want to explore different approaches:
- **branch_id**: Give your thought branch a unique identifier
- **branch_from_step**: Specify which step number serves as your branching point

### Revisions
Question or modify previous steps as your understanding evolves:
- **is_step_revision**: Set to true when revising previous thinking
- **revises_step**: Indicate which step number you're reconsidering

### Visualization
The tool provides a visual representation of your thinking process:
- Completed steps: [✓ 1]
- Revised steps: [🔄 2]
- Upcoming steps: [ 3 ]
- Branch indicators: → [Branch: alternative_approach]

## How You Should Use This Tool

- Start with an initial estimate of needed steps, but be ready to adjust
- Feel free to question or revise your previous thinking
- Don't hesitate to add more steps even after reaching what seemed like the end
- Express uncertainty when present
- Mark steps that revise previous thinking or branch into new paths
- Ignore information that is irrelevant to your current step
- Only set next_step_needed to false when truly done
"#;
    type Args = WorkflowStep;

    async fn call(&self, args: Self::Args) -> Result<CallToolResult, ToolError> {
        // Check if max_steps is reached
        if let Some(max) = self.max_steps {
            if args.step_number > max {
                return Ok(Self::error(format!(
                    "Step number {} exceeds configured maximum of {}",
                    args.step_number, max
                )));
            }
        }

        let mut state = self.state.lock().await;

        // Adjust total_steps if needed
        let mut step_data = args.clone();
        if step_data.step_number > step_data.total_steps {
            step_data.total_steps = step_data.step_number;
        }

        // Validate inputs
        if step_data.revises_step.is_some() && step_data.is_step_revision.is_none() {
            return Ok(Self::error("When specifying revises_step, is_step_revision must be set to true"));
        }

        if step_data.branch_id.is_some() && step_data.branch_from_step.is_none() {
            return Ok(Self::error("When creating a branch (branch_id), you must specify branch_from_step"));
        }

        // Handle branch logic
        if let (Some(branch_id), Some(branch_from_step)) = (&step_data.branch_id, &step_data.branch_from_step) {
            if !self.allow_branches {
                return Ok(Self::error("Branching is disabled in current configuration"));
            }

            // Validate the branch_from_step exists
            if *branch_from_step <= 0 || *branch_from_step > state.step_history.len() as i32 {
                return Ok(Self::error(format!(
                    "branch_from_step {} does not exist in step history",
                    branch_from_step
                )));
            }

            state.current_branch = Some(branch_id.clone());
            state.branches.entry(branch_id.clone()).or_default().push(step_data.clone());
        } else if state.current_branch.is_some() && step_data.branch_id.is_none() {
            state.current_branch = None;
        }

        // Add to step history
        state.step_history.push(step_data.clone());

        // Build response data
        let response = self.build_workflow_status(&state, &step_data).await;

        // Convert response to JSON string with pretty formatting
        match serde_json::to_string_pretty(&response) {
            Ok(json_response) => Ok(Self::success(json_response)),
            Err(e) => Ok(Self::error(format!("Failed to serialize response: {}", e))),
        }
    }
}

impl Workflow {
    /// Creates a new Workflow with custom configuration
    pub fn new(allow_branches: bool, max_steps: Option<i32>) -> Self {
        Self { state: Arc::new(Mutex::new(WorkflowState::default())), allow_branches, max_steps }
    }

    /// Generate a simple text visualization of workflow progress
    fn format_workflow_progress(&self, steps: &[WorkflowStep], total_steps: i32) -> String {
        let mut result = String::new();

        for i in 1..=total_steps {
            let step = steps.iter().find(|s| s.step_number == i);

            if let Some(s) = step {
                if s.is_step_revision == Some(true) {
                    result.push_str(&format!("[🔄 {}] → ", i));
                } else {
                    result.push_str(&format!("[✓ {}] → ", i));
                }
            } else {
                result.push_str(&format!("[ {} ] → ", i));
            }
        }

        // Remove trailing arrow
        if result.ends_with(" → ") {
            result.truncate(result.len() - 4);
        }

        result
    }

    /// Retrieve steps based on query parameters
    pub async fn get_steps(&self, query: StepQuery) -> Result<Vec<WorkflowStep>, ToolError> {
        let state = self.state.lock().await;

        // Handle different query types
        if let Some(step_num) = query.step_number {
            // Return a specific step
            return Ok(state.step_history.iter().filter(|s| s.step_number == step_num).cloned().collect());
        } else if let Some(branch_id) = query.branch_id {
            // Return steps from a specific branch
            if let Some(branch_steps) = state.branches.get(&branch_id) {
                return Ok(branch_steps.clone());
            } else {
                return Ok(Vec::new());
            }
        } else if let Some(n) = query.return_last_n {
            // Return the last N steps
            let n = n as usize;
            if n >= state.step_history.len() {
                return Ok(state.step_history.clone());
            } else {
                return Ok(state.step_history[state.step_history.len() - n..].to_vec());
            }
        }

        // Default: return all steps
        Ok(state.step_history.clone())
    }

    // Add these helper methods similar to fetch.rs
    fn error(error_message: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::to_value(TextContent {
                type_: "text".to_string(),
                text: error_message.into(),
                annotations: None,
            })
            .unwrap()],
            is_error: Some(true),
            meta: None,
        }
    }

    fn success(message: impl Into<String>) -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::to_value(TextContent {
                type_: "text".to_string(),
                text: message.into(),
                annotations: None,
            })
            .unwrap()],
            is_error: Some(false),
            meta: None,
        }
    }

    // Add this new helper method to build the workflow status
    async fn build_workflow_status(&self, state: &WorkflowState, step_data: &WorkflowStep) -> WorkflowStatus {
        // Build branch status map
        let mut active_branches = HashMap::new();
        for (branch_name, branch_steps) in &state.branches {
            if let Some(first_step) = branch_steps.first() {
                let branch_from = first_step.branch_from_step.unwrap_or(0);
                active_branches.insert(
                    branch_name.clone(),
                    BranchStatus {
                        branch_name: branch_name.clone(),
                        created_at_step: branch_from,
                        steps_in_branch: branch_steps.len() as i32,
                        is_active: Some(branch_name) == state.current_branch.as_ref(),
                    },
                );
            }
        }

        // Get recent steps (last 3)
        let recent_steps = if state.step_history.len() <= 3 {
            state.step_history.clone()
        } else {
            state.step_history[state.step_history.len() - 3..].to_vec()
        };

        WorkflowStatus {
            step_number: step_data.step_number,
            total_steps: step_data.total_steps,
            next_step_needed: step_data.next_step_needed,
            last_step_description: step_data.step_description.clone(),
            current_branch: state.current_branch.clone(),
            branches: state.branches.keys().cloned().collect(),
            step_history_length: state.step_history.len(),
            recent_steps,
            active_branches,
            progress_visualization: self.format_workflow_progress(&state.step_history, step_data.total_steps),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolDef;

    #[tokio::test]
    async fn test_workflow_tool() {
        let tool = Workflow::default();
        let step = WorkflowStep {
            step_description: "Initial step".to_string(),
            step_number: 1,
            total_steps: 3,
            next_step_needed: true,
            is_step_revision: None,
            revises_step: None,
            branch_from_step: None,
            branch_id: None,
            needs_more_steps: None,
        };

        let result = ToolDef::call(&tool, step).await.unwrap();
        let content = result.content[0]["text"].as_str().unwrap();
        let response: WorkflowStatus = serde_json::from_str(content).unwrap();

        assert_eq!(response.step_number, 1);
        assert_eq!(response.total_steps, 3);
        assert_eq!(response.next_step_needed, true);
        assert_eq!(response.step_history_length, 1);
        assert!(response.branches.is_empty());
    }

    #[tokio::test]
    async fn test_workflow_branching() {
        let tool = Workflow::default();

        // Add main flow step 1
        let step1 = WorkflowStep {
            step_description: "Initial step".to_string(),
            step_number: 1,
            total_steps: 3,
            next_step_needed: true,
            is_step_revision: None,
            revises_step: None,
            branch_from_step: None,
            branch_id: None,
            needs_more_steps: None,
        };
        let _ = ToolDef::call(&tool, step1).await.unwrap();

        // Add branch
        let branch_step = WorkflowStep {
            step_description: "Branch step".to_string(),
            step_number: 2,
            total_steps: 3,
            next_step_needed: true,
            is_step_revision: None,
            revises_step: None,
            branch_from_step: Some(1),
            branch_id: Some("test_branch".to_string()),
            needs_more_steps: None,
        };

        let result = ToolDef::call(&tool, branch_step).await.unwrap();
        let content = result.content[0]["text"].as_str().unwrap();
        let response: WorkflowStatus = serde_json::from_str(content).unwrap();

        assert_eq!(response.current_branch, Some("test_branch".to_string()));
        assert_eq!(response.branches.len(), 1);
        assert!(response.branches.contains(&"test_branch".to_string()));
    }

    #[tokio::test]
    async fn test_step_retrieval() {
        let tool = Workflow::default();

        // Add steps
        for i in 1..=5 {
            let step = WorkflowStep {
                step_description: format!("Step {}", i),
                step_number: i,
                total_steps: 5,
                next_step_needed: i < 5,
                is_step_revision: None,
                revises_step: None,
                branch_from_step: None,
                branch_id: None,
                needs_more_steps: None,
            };
            let _ = ToolDef::call(&tool, step).await.unwrap();
        }

        // Test getting a specific step
        let query = StepQuery { step_number: Some(3), branch_id: None, return_last_n: None };

        let steps = tool.get_steps(query).await.unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_number, 3);

        // Test getting last steps
        let query = StepQuery { step_number: None, branch_id: None, return_last_n: Some(2) };

        let steps = tool.get_steps(query).await.unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].step_number, 4);
        assert_eq!(steps[1].step_number, 5);
    }

    #[test]
    fn test_serialization() {
        let tool = Workflow::new(true, Some(10));
        let serialized = serde_json::to_string(&tool).unwrap();
        let deserialized: Workflow = serde_json::from_str(&serialized).unwrap();

        assert_eq!(tool.allow_branches, deserialized.allow_branches);
        assert_eq!(tool.max_steps, deserialized.max_steps);
    }
}
