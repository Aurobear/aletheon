import json

from src.lab.diagnostics import diagnose, validate_analysis
from src.lab.model import DiagnosticSettings
from src.lab.redaction import redact


class _Response:
    def __init__(self, payload):
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return None

    def read(self, _limit):
        return json.dumps(self.payload).encode()


def _analysis():
    return {
        "schema_version": 1,
        "classification": "product_failure",
        "confidence": 0.9,
        "summary": "The deterministic command failed.",
        "hypotheses": [
            {
                "claim": "The assertion is violated.",
                "support": ["result.json"],
                "disconfirming_test": "Run the case against the parent SHA.",
            }
        ],
        "suspected_components": ["executive"],
        "evidence_refs": ["result.json"],
    }


def test_redaction_removes_known_and_pattern_secrets():
    text, count = redact(
        "api_key=visible-secret Authorization: BearerValue raw-visible-secret",
        known_secrets=("raw-visible-secret",),
    )
    assert "visible-secret" not in text
    assert "BearerValue" not in text
    assert "raw-visible-secret" not in text
    assert count == 3


def test_diagnose_sends_redacted_evidence_and_validates_json(monkeypatch):
    monkeypatch.setenv("LEJU_API_KEY", "test-secret-value")
    captured = {}

    def opener(request, timeout):
        captured["authorization"] = request.headers["Authorization"]
        captured["body"] = request.data.decode()
        captured["timeout"] = timeout
        return _Response(
            {"choices": [{"message": {"content": json.dumps(_analysis())}}]}
        )

    result = diagnose(
        DiagnosticSettings(enabled=True),
        {"stderr": "api_key=test-secret-value"},
        opener=opener,
    )
    assert result["status"] == "completed"
    assert result["classification"] == "product_failure"
    assert result["provider"]["redaction_count"] >= 1
    assert "test-secret-value" not in captured["body"]
    assert captured["authorization"] == "Bearer test-secret-value"


def test_diagnose_fails_closed_without_credential(monkeypatch):
    monkeypatch.delenv("LEJU_API_KEY", raising=False)
    result = diagnose(DiagnosticSettings(enabled=True), {"failure": "x"})
    assert result["status"] == "infra_blocked"


def test_invalid_model_analysis_is_not_accepted(monkeypatch):
    monkeypatch.setenv("LEJU_API_KEY", "test-secret-value")

    def opener(_request, _timeout):
        return _Response({"choices": [{"message": {"content": "not-json"}}]})

    result = diagnose(DiagnosticSettings(enabled=True), {"failure": "x"}, opener=opener)
    assert result["status"] == "analysis_invalid"


def test_analysis_confidence_cannot_be_boolean():
    value = _analysis()
    value["confidence"] = True
    try:
        validate_analysis(value)
    except ValueError as error:
        assert "confidence" in str(error)
    else:
        raise AssertionError("boolean confidence was accepted")


def test_analysis_rejects_unknown_evidence_reference():
    value = _analysis()
    value["evidence_refs"] = ["invented.log"]
    try:
        validate_analysis(value)
    except ValueError as error:
        assert "unknown" in str(error)
    else:
        raise AssertionError("unknown evidence reference was accepted")
