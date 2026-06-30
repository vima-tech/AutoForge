# Conversation Orchestration

## Purpose

Conversation orchestration turns natural group-chat requests into durable multi-Agent tasks.
The chat page stays conversational: users send messages and mention Agents; the backend owns planning,
execution order, concurrency, task state, and message persistence.

## Concepts

- Agent: a reusable participant that can be assigned different responsibilities such as Planner, analysis, testing, architecture review, or document writing.
- System role assignment: assigning an Agent to a system role enables that role; clearing the assignment disables it.
- Supported system roles: `planner`, `summarizer`, `doc_writer`, `context_compressor`.
- Conversation Task: one user-triggered orchestration run.
- Task Step: one serial stage in a task. A step is either `parallel` or `single`.
- Task Run: one Agent execution inside one step.
- Agent LLM: every Agent execution uses that Agent's own `llm_id`. The Planner never assigns or overrides the LLM used by other Agents.

## Data Model

Agent metadata:

- `system_kind`: optional comma-separated assignment markers such as `planner,doc_writer`
- `visible_in_chat`: whether the Agent gets direct conversations and appears in group member pickers
- `mentionable`: whether the Agent can be selected by chat orchestration
- `enabled`: whether the runtime can execute the Agent

Task tables:

- `conversation_tasks`: top-level orchestration run
- `conversation_task_steps`: ordered serial steps
- `conversation_task_runs`: per-Agent execution records

## Execution Rules

1. The frontend persists the user's message first.
2. The frontend calls `start_conversation_task` with the trigger message id, raw instruction, explicit mention ids, and context window.
3. The backend creates a task row and returns immediately.
4. The async runtime builds one context snapshot from the conversation history.
5. If the request is a direct conversation, the direct Agent gets a single step.
6. If explicit mentions are present and the request is simple, the mentioned Agents run in one step.
7. If the request requires sequencing or has no explicit mention, the Planner Agent produces a JSON plan.
8. Parallel step Agents all receive the same context snapshot plus completed prior-step output.
9. Each Agent writes its own message as soon as it finishes.
10. The next step starts only after the current step finishes.
11. Each Agent is executed through its configured LLM adapter: Anthropic native, Ollama, OpenAI-compatible/custom, or Claude CLI fallback when no LLM is configured.

## Planner Output

The Planner must output JSON only:

```json
{
  "steps": [
    {
      "type": "parallel",
      "agents": ["agent-analyst", "agent-architect"],
      "instruction": "分别从需求和架构角度评审"
    },
    {
      "type": "single",
      "agents": ["agent-analyst"],
      "instruction": "综合以上意见输出 PRD 草案"
    }
  ]
}
```

The runtime validates all Agent ids against the current conversation members and drops invalid entries.

## Failure Semantics

- Agent execution failure is written as that Agent's chat message with a system error.
- The corresponding `conversation_task_runs` row is marked `failed`.
- A step is `failed` if any run failed.
- The task is `failed` if any step failed, but successful Agent outputs remain in the conversation.

## Extension Points

- Summarizer and Doc Writer can be added as future assignments using the same Agent table and LLM selector.
- `artifact` message blocks should be used for PRD, ADR, test plan, and implementation-plan outputs.
- Future provider-specific LLM adapters should sit behind the Agent execution path, not the chat page.
