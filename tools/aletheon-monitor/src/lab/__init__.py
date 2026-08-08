"""External continuous-engineering supervisor for Aletheon.

The lab intentionally runs outside the Aletheon process.  Product events are
evidence, but process lifecycle, cleanup and the final case verdict remain
host-owned facts.
"""

from .model import CaseSpec, DiagnosticSettings, LabSettings

__all__ = ["CaseSpec", "DiagnosticSettings", "LabSettings"]
