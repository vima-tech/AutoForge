"""
Claude Code Agent — builds execution prompt and runs claude CLI in a worktree.
Design doc §10.2.
"""
import asyncio
import json
from pathlib import Path


CODING_SPEC_PATH = Path(__file__).parent.parent.parent / "specs" / "coding-spec.md"


def build_prompt(
    *,
    issue_title: str,
    issue_description: str | None,
    analysis_summary: str | None,
    implementation_hints: str | None,
    admin_suggestions: str | None,
    project_config: dict,
    iteration: int = 1,
) -> str:
    """Build the full prompt for Claude Code to execute in the worktree."""
    conventions = project_config.get("claude_context", {}).get("conventions", "")
    forbidden_paths = project_config.get("claude_context", {}).get("forbidden_paths", [])
    key_files = project_config.get("claude_context", {}).get("key_files", [])

    try:
        coding_spec = CODING_SPEC_PATH.read_text()
    except FileNotFoundError:
        coding_spec = "（编码规范文档未找到，请遵循最佳实践）"

    forbidden_str = "\n".join(f"  - {p}" for p in forbidden_paths) or "  无"
    key_files_str = "\n".join(f"  - {p}" for p in key_files) or "  无"

    iteration_note = ""
    if iteration > 1:
        iteration_note = f"\n⚠ 这是第 {iteration} 次迭代。请仔细阅读管理员的修改建议，重点解决上一轮的问题。\n"

    prompt = f"""# AutoForge 任务{iteration_note}

## 需求
**标题：** {issue_title}
**描述：** {issue_description or '（无详细描述）'}

## 分析报告摘要
{analysis_summary or '（无分析摘要）'}

## 实现方向建议
{implementation_hints or '（无具体建议）'}

## 管理员附加要求
{admin_suggestions or '（无附加要求）'}

## 项目约定
{conventions}

## 关键参考文件
{key_files_str}

## 禁止修改的路径
{forbidden_str}

## 编码规范
{coding_spec}

---

**任务指令：**
1. 阅读上述需求和规范
2. 找到需要修改的相关代码
3. 实现改动，范围最小化
4. 运行项目测试确认无回归
5. 生成实现报告（格式见编码规范）

请开始执行。"""

    return prompt


async def run_claude_code(
    *,
    worktree_path: str,
    prompt: str,
    timeout_seconds: int = 1800,
    env_extra: dict | None = None,
) -> tuple[int, str, str]:
    """
    Run claude CLI in the worktree directory.
    Returns (exit_code, stdout, stderr).
    """
    import os
    env = os.environ.copy()
    if env_extra:
        env.update(env_extra)

    # Write prompt to a temp file to avoid shell escaping issues
    prompt_file = Path(worktree_path) / ".autoforge_prompt.md"
    prompt_file.write_text(prompt, encoding="utf-8")

    try:
        proc = await asyncio.create_subprocess_exec(
            "claude",
            "--print",  # non-interactive, print output and exit
            f"@{prompt_file}",  # read prompt from file
            cwd=worktree_path,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=env,
        )
        stdout_bytes, stderr_bytes = await asyncio.wait_for(
            proc.communicate(), timeout=timeout_seconds
        )
        return proc.returncode or 0, stdout_bytes.decode(), stderr_bytes.decode()

    except asyncio.TimeoutError:
        proc.kill()
        return -1, "", f"Claude Code execution timed out after {timeout_seconds}s"
    finally:
        prompt_file.unlink(missing_ok=True)
