import asyncio
import shutil
import os

import pytest

from src import tui_session as ts

pytestmark = pytest.mark.skipif(
    shutil.which("tmux") is None, reason="tmux not installed"
)


def test_start_send_capture_stop():
    async def scenario():
        s = "aletheon-tui-smoke"
        started = await ts.start("cat", session=s, cols=80, rows=20)
        assert started["ok"], started
        assert started["working_dir"] == os.path.realpath(os.getcwd())
        await ts.send("hello-tui", session=s, submit=True)
        await asyncio.sleep(0.3)
        frame = await ts.capture(session=s)
        assert "hello-tui" in frame
        killed = await ts.kill(session=s)
        assert killed["ok"]
        assert await ts.has_session(session=s) is False

    asyncio.run(scenario())
