"""
Test Agent — runs test suites and parses results.
Handles both reactive (post-merge) and proactive (scheduled) modes.
Design doc §10.3.
"""
import asyncio
import re
from dataclasses import dataclass


@dataclass
class TestResult:
    passed: int
    failed: int
    errors: int
    duration_seconds: float
    coverage_percent: float | None
    failures: list[dict]  # list of {test_name, error_message, file_path, line_number}
    raw_output: str


async def run_test_suite(
    *,
    command: str,
    cwd: str,
    timeout: int = 300,
    env_extra: dict | None = None,
) -> TestResult:
    """Execute a test command and parse the output."""
    import os
    env = os.environ.copy()
    if env_extra:
        env.update(env_extra)

    proc = await asyncio.create_subprocess_shell(
        command,
        cwd=cwd,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
        env=env,
    )

    try:
        stdout_bytes, _ = await asyncio.wait_for(proc.communicate(), timeout=timeout)
    except asyncio.TimeoutError:
        proc.kill()
        return TestResult(
            passed=0, failed=0, errors=1, duration_seconds=timeout,
            coverage_percent=None,
            failures=[{"test_name": "timeout", "error_message": f"Test suite timed out after {timeout}s"}],
            raw_output=f"TIMEOUT after {timeout}s",
        )

    output = stdout_bytes.decode(errors="replace")
    return _parse_pytest_output(output, proc.returncode or 0)


def _parse_pytest_output(output: str, exit_code: int) -> TestResult:
    """Parse pytest --tb=short output."""
    # Summary line: "5 passed, 2 failed, 1 error in 3.14s"
    summary_pattern = re.compile(
        r"(?:(\d+) passed)?.*?(?:(\d+) failed)?.*?(?:(\d+) error)?.*?in ([\d.]+)s",
        re.IGNORECASE,
    )
    match = summary_pattern.search(output)

    passed = int(match.group(1) or 0) if match else 0
    failed = int(match.group(2) or 0) if match else 0
    errors = int(match.group(3) or 0) if match else 0
    duration = float(match.group(4) or 0) if match else 0.0

    # If exit_code != 0 but no failures parsed, mark as error
    if exit_code != 0 and passed == 0 and failed == 0 and errors == 0:
        errors = 1

    # Coverage: "TOTAL  ... 87%"
    cov_match = re.search(r"TOTAL\s+\d+\s+\d+\s+(\d+)%", output)
    coverage = float(cov_match.group(1)) / 100 if cov_match else None

    # Parse individual failures
    failures = []
    fail_blocks = re.split(r"\n_{3,}\s*FAILED\s+", output)
    for block in fail_blocks[1:]:
        lines = block.strip().split("\n")
        test_name = lines[0].strip() if lines else "unknown"
        error_lines = [l for l in lines if l.startswith("E ")]
        error_msg = error_lines[0][2:].strip() if error_lines else "Unknown error"

        # Try to extract file/line
        loc_match = re.search(r"(\S+\.py):(\d+):", block)
        failures.append({
            "test_name": test_name,
            "error_message": error_msg,
            "file_path": loc_match.group(1) if loc_match else None,
            "line_number": int(loc_match.group(2)) if loc_match else None,
        })

    return TestResult(
        passed=passed,
        failed=failed,
        errors=errors,
        duration_seconds=duration,
        coverage_percent=coverage,
        failures=failures,
        raw_output=output[:20000],  # cap storage
    )


async def run_quality_checks(
    *,
    quality_config: dict,
    cwd: str,
    timeout: int = 120,
) -> dict[str, dict]:
    """Run lint, typing, security checks. Returns {check_name: {exit_code, output}}."""
    results = {}
    checks = {
        "lint": quality_config.get("lint"),
        "typing": quality_config.get("typing"),
        "security": quality_config.get("security"),
    }
    for name, cmd in checks.items():
        if not cmd:
            continue
        proc = await asyncio.create_subprocess_shell(
            cmd,
            cwd=cwd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.STDOUT,
        )
        try:
            stdout_bytes, _ = await asyncio.wait_for(proc.communicate(), timeout=timeout)
            results[name] = {
                "exit_code": proc.returncode or 0,
                "output": stdout_bytes.decode(errors="replace")[:5000],
            }
        except asyncio.TimeoutError:
            proc.kill()
            results[name] = {"exit_code": -1, "output": f"Timed out after {timeout}s"}

    return results
