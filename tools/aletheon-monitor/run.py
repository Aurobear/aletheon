#!/usr/bin/env python3
"""Aletheon Monitor MCP Server — standalone entry point.

Usage:
    python3 tools/aletheon-monitor/run.py

Configured via environment:
    ALETHEON_SOCKET  — daemon socket path (default: /run/aletheon/aletheon.sock)
    ALETHEON_TIMEOUT — RPC timeout in seconds (default: 5)
"""

import sys
import os

# Ensure the aletheon-monitor package is importable
_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

from src.server import main
import asyncio

asyncio.run(main())
