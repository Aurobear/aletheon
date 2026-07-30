#!/usr/bin/env python3
import pathlib, sys, tomllib
root = pathlib.Path(__file__).resolve().parents[1]
path = root / "config/approval-closure.toml"
data = tomllib.loads(path.read_text())
assert data["format_version"] == 1 and data["default_policy"] == "deny"
rows = data["operation"]
names = {row["name"] for row in rows}
assert len(names) == len(rows)
required_categories = {"apply_code", "activate_goal", "send_mail", "delete_file",
 "modify_calendar", "git_push", "capability_expansion", "dasein_modification", "budget_expansion"}
assert {row["category"] for row in rows} == required_categories
for row in rows:
    assert row["status"] in {"closed", "denied"}
    if row["status"] == "closed":
        for key in ("producer", "resolver", "receipt", "replay_test"):
            candidate = (root / row[key]).resolve()
            assert root in candidate.parents and candidate.is_file(), (row["name"], key)
    else:
        assert not any(key in row for key in ("producer", "resolver", "receipt")), row["name"]
print(f"approval closure matrix verified: closed={sum(r['status']=='closed' for r in rows)} denied={sum(r['status']=='denied' for r in rows)}")
