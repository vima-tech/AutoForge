"""
autoforge.yaml parser with Pydantic validation.
Design doc §2.1.
"""
from typing import Literal
from pydantic import BaseModel, Field


class BuildConfig(BaseModel):
    command: str
    timeout: int = 120


class StartConfig(BaseModel):
    command: str
    port: int = 8000
    health_check: str = "GET /health"
    ready_timeout: int = 30


class SeedConfig(BaseModel):
    command: str


class SensitiveField(BaseModel):
    table: str
    fields: list[str]
    rule: Literal["mask", "hash", "drop"] = "mask"


class PreviewConfig(BaseModel):
    build: BuildConfig | None = None
    start: StartConfig | None = None
    env: dict[str, str] = Field(default_factory=dict)
    seed: SeedConfig | None = None
    sensitive_fields: list[SensitiveField] = Field(default_factory=list)


class SuiteConfig(BaseModel):
    command: str
    timeout: int = 300


class TestConfig(BaseModel):
    unit: SuiteConfig | None = None
    integration: SuiteConfig | None = None
    coverage: dict | None = None


class QualityConfig(BaseModel):
    lint: str | None = None
    typing: str | None = None
    security: str | None = None


class ClaudeContextConfig(BaseModel):
    key_files: list[str] = Field(default_factory=list)
    forbidden_paths: list[str] = Field(default_factory=list)
    conventions: str = ""


class BranchesConfig(BaseModel):
    dev: str = "dev"
    main: str = "main"
    worktree_prefix: str = "autoforge/cr-"


class FeedbackSource(BaseModel):
    type: str
    repo: str | None = None
    path: str | None = None


class ConcurrencyConfig(BaseModel):
    max_slots: int = 5
    pause_threshold: int = 20
    queue_strategy: Literal["fifo", "priority"] = "fifo"


class ProjectInfo(BaseModel):
    name: str
    description: str = ""
    language: str = ""
    framework: str = ""


class AutoForgeConfig(BaseModel):
    project: ProjectInfo
    preview: PreviewConfig = Field(default_factory=PreviewConfig)
    test: TestConfig = Field(default_factory=TestConfig)
    quality: QualityConfig = Field(default_factory=QualityConfig)
    claude_context: ClaudeContextConfig = Field(default_factory=ClaudeContextConfig)
    branches: BranchesConfig = Field(default_factory=BranchesConfig)
    feedback_sources: list[FeedbackSource] = Field(default_factory=list)
    concurrency: ConcurrencyConfig = Field(default_factory=ConcurrencyConfig)
    docker_image: str = ""
    execution_timeout: int = 1800


def parse_config(yaml_content: str) -> AutoForgeConfig:
    """Parse autoforge.yaml content into a validated config object."""
    import yaml
    data = yaml.safe_load(yaml_content)
    return AutoForgeConfig.model_validate(data)


def config_to_dict(config: AutoForgeConfig) -> dict:
    return config.model_dump()


def load_config_from_repo(repo_path: str) -> AutoForgeConfig | None:
    """Load autoforge.yaml from a local repository path."""
    from pathlib import Path
    config_file = Path(repo_path) / "autoforge.yaml"
    if not config_file.exists():
        return None
    return parse_config(config_file.read_text())
