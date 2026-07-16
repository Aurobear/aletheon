"""Monitor MCP server and installed production-scenario entry point."""
from __future__ import annotations
import asyncio
import sys

if len(sys.argv) > 1 and sys.argv[1] == "scenario":
    from . import scenarios
    sys.argv.pop(1)
    scenarios.main()
else:
    from .server import main
    asyncio.run(main())
