# Gmail Goal ingress configuration

Gmail synchronization remains read-only. Goal intake is disabled unless the
daemon has an active `gmail.readonly` account binding and
`ALETHEON_GMAIL_INGRESS_POLICY_FILE` names a valid JSON policy file.

Each entry is bound to the persisted Google account UUID and its owning
principal. Missing accounts, owner mismatches, invalid policies, and senders
outside the allowlist fail closed.

```json
{
  "policies": [
    {
      "account_id": "00000000-0000-0000-0000-000000000000",
      "principal": "owner-principal",
      "version": 1,
      "allowed_addresses": ["trusted-sender@example.com"],
      "allowed_domains": [],
      "trusted_authserv_ids": ["mx.google.com"],
      "authentication": "spf_or_dkim"
    }
  ]
}
```

Supported authentication values are `spf_or_dkim`, `spf`, `dkim`, and
`spf_and_dkim`. Increment `version` whenever the policy changes. Exact sender
addresses and domains must be lowercase canonical ASCII values. The trusted
authentication-service ID must identify the receiving-chain
`Authentication-Results` header; do not copy a value from message body text.

After setting the environment variable, restart the daemon. A verified exact
`[GOAL]` subject creates a non-executable Draft and a durable Telegram review.
It cannot execute until the bound Telegram owner confirms it. Other subjects
remain notifications/quarantine according to channel policy. Gmail send and
Calendar write capabilities are not enabled by this configuration.
