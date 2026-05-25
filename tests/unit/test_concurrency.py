import pytest
from autoforge.core.concurrency import ConcurrencyManager, SystemStage


@pytest.fixture
def mgr():
    return ConcurrencyManager(max_slots=5, pause_threshold=20)


def test_initial_stage_is_normal(mgr):
    assert mgr.get_stage() == SystemStage.NORMAL


def test_effective_concurrency_normal(mgr):
    assert mgr.effective_concurrency == 5


@pytest.mark.asyncio
async def test_acquire_slot(mgr):
    result = await mgr.acquire_slot("cr-1")
    assert result is True
    assert mgr._active_slots == 1


@pytest.mark.asyncio
async def test_throttled_when_all_slots_in_pending_review(mgr):
    # Fill all slots with pending review items
    mgr._active_slots = 5
    mgr._pending_review_count = 5
    assert mgr.get_stage() == SystemStage.THROTTLED
    assert mgr.effective_concurrency == 1


@pytest.mark.asyncio
async def test_paused_at_threshold(mgr):
    mgr._pending_review_count = 20
    assert mgr.get_stage() == SystemStage.PAUSED
    assert mgr.effective_concurrency == 0
    result = await mgr.acquire_slot("cr-x")
    assert result is False


@pytest.mark.asyncio
async def test_release_slot(mgr):
    mgr._active_slots = 3
    mgr._pending_review_count = 2
    await mgr.release_slot()
    assert mgr._active_slots == 2
    assert mgr._pending_review_count == 1
