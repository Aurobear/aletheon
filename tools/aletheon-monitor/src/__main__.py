"""Entry point for python -m aletheon_monitor."""
import asyncio
from .server import main

asyncio.run(main())
