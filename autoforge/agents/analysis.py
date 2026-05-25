"""
Analysis Agent — evaluates issues and produces structured analysis reports.
Design doc §10.1 and analysis-spec.md.
Uses claude-haiku for cost efficiency on high-volume analysis tasks.
"""
import json
from anthropic import AsyncAnthropic
from autoforge.config import settings

_client = AsyncAnthropic(api_key=settings.anthropic_api_key)

ANALYSIS_MODEL = "claude-haiku-4-5-20251001"

SYSTEM_PROMPT = """You are an expert software project analyst for AutoForge, an autonomous software factory.
Your job is to analyze incoming issue reports and evaluate them according to the analysis specification.

You must respond with a valid JSON object and nothing else. No markdown, no explanation outside the JSON."""

ANALYSIS_PROMPT_TEMPLATE = """Analyze the following issue and produce a structured evaluation.

Issue:
Title: {title}
Description: {description}
Source: {source_type}
Category: {category}
Severity: {severity}

Project context:
{project_context}

Existing issues (for duplicate detection):
{existing_issues_summary}

Respond with this exact JSON structure:
{{
  "authenticity_score": <float 0.0-1.0, how genuine/real this issue is>,
  "feasibility_score": <float 0.0-1.0, how implementable in current tech stack>,
  "priority_suggestion": <int 1-10>,
  "category_suggestion": <"Bug"|"Feature"|"Improvement"|"Security"|"Debt">,
  "severity_suggestion": <"critical"|"high"|"medium"|"low">,
  "duplicate_of_fingerprint": <string|null, fingerprint of likely duplicate issue or null>,
  "affected_modules": <list of strings, estimated modules affected>,
  "analysis_summary": <string, ≤200 chars, concise summary in Chinese>,
  "implementation_hints": <string, ≤300 chars, key implementation direction in Chinese>,
  "risks": <string|null, potential risks in Chinese, null if none>,
  "spec_gaps": <string|null, any analysis-spec gaps encountered in Chinese, null if none>
}}"""


async def analyze_issue(
    *,
    title: str,
    description: str | None,
    source_type: str,
    category: str | None,
    severity: str | None,
    project_context: str = "",
    existing_issues_summary: str = "无",
) -> dict:
    """
    Run the analysis agent and return parsed results.
    Raises on API error; caller handles retry via Celery.
    """
    prompt = ANALYSIS_PROMPT_TEMPLATE.format(
        title=title,
        description=description or "（无详细描述）",
        source_type=source_type,
        category=category or "未分类",
        severity=severity or "未知",
        project_context=project_context or "无额外上下文",
        existing_issues_summary=existing_issues_summary,
    )

    response = await _client.messages.create(
        model=ANALYSIS_MODEL,
        max_tokens=800,
        system=SYSTEM_PROMPT,
        messages=[{"role": "user", "content": prompt}],
    )

    raw = response.content[0].text.strip()

    # Strip markdown fences if model adds them
    if raw.startswith("```"):
        raw = raw.split("```")[1]
        if raw.startswith("json"):
            raw = raw[4:]
    raw = raw.strip()

    result = json.loads(raw)
    result["raw_llm_output"] = {"model": ANALYSIS_MODEL, "response": raw}
    return result
