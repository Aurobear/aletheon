"""Ensure the aletheon-monitor package root is importable as `src.*`."""
import os
import sys

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Editable installs add ``src/`` ahead of the repository root. That makes
# ``import scenarios`` resolve the monitor's ``src/scenarios.py`` module
# instead of the acceptance-scenario package. Keep the checkout root first so
# the suite tests the source tree deterministically after deployment.
while _ROOT in sys.path:
    sys.path.remove(_ROOT)
sys.path.insert(0, _ROOT)
