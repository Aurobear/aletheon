# Private administration boundary

Aletheon administration is local-first. The daemon, MCP bridge, and health RPC
remain on `/run/aletheon/aletheon.sock`; optional GBrain ports bind only to
`127.0.0.1`. Do not add a public HTTP health endpoint. Operators use SSH/SFTP or
`scp` over Tailscale. Telegram remains an outbound long-polling client.

```text
owner device -- encrypted tailnet --> tailscale0:22 (SSH)
                                      |
                                      +--> local Unix socket --> Aletheon
internet/LAN/unapproved tailnet --X--> host administration
```

## Tailnet policy

Tag the server `tag:aletheon-server` and owner devices
`tag:aletheon-operator`. Only a small owner-admin group may assign those tags.
A representative tailnet policy is:

```json
{
  "tagOwners": {
    "tag:aletheon-server": ["group:aletheon-admins"],
    "tag:aletheon-operator": ["group:aletheon-admins"]
  },
  "acls": [
    {"action":"accept","src":["tag:aletheon-operator"],
     "dst":["tag:aletheon-server:22"]}
  ],
  "ssh": [
    {"action":"check","src":["tag:aletheon-operator"],
     "dst":["tag:aletheon-server"],"users":["autogroup:nonroot"]}
  ]
}
```

Require device approval and MFA, expire operator devices, disable reusable auth
keys, and review nodes monthly. For a lost device: remove it from the tailnet,
revoke its auth key/session, rotate any copied host key, and review SSH/audit
events. Keep one offline recovery code and a console-capable recovery account in
a separately controlled vault. Recovery access must not add a permanent public
listener.

## Host firewall

The host input policy is drop. Permit loopback, established/related traffic,
ICMP needed for path discovery, Tailscale transport, and SSH arriving on
`tailscale0`; deny SSH from LAN/WAN interfaces. Outbound policy permits
established traffic and required HTTPS, DNS, NTP, Telegram, Google/provider, and
Tailscale coordination/DERP. Prefer destination-aware egress controls where
operationally maintainable.

Example nftables policy fragment (adapt interface names and Tailscale's current
transport requirements before applying through the host's existing firewall):

```nft
table inet filter {
  chain input {
    type filter hook input priority 0; policy drop;
    iifname "lo" accept
    ct state established,related accept
    ip protocol icmp accept
    ip6 nexthdr ipv6-icmp accept
    iifname "tailscale0" tcp dport 22 accept
    udp dport 41641 accept
  }
  chain forward { type filter hook forward priority 0; policy drop; }
  chain output { type filter hook output priority 0; policy accept; }
}
```

## Google OAuth

Use a loopback redirect such as `http://127.0.0.1:<random>/callback` and perform
the authenticated owner flow through an SSH port forward. The callback listener
must bind `127.0.0.1`, validate state and PKCE, have a short timeout, and close
after one result. Do not publish it through Funnel, a reverse proxy, LAN bind, or
public DNS. If loopback forwarding is impossible, a documented emergency flow
may bind briefly to the server's Tailscale address, allow only the operator tag,
and remove both listener and ACL immediately after completion.

## Evidence drill

Run `verify-network-exposure.sh --strict`, save redacted `ss -lntup`, firewall
rules, and `tailscale status --json` summaries, then test:

1. Localhost can reach the Unix socket; there is no Aletheon TCP listener.
2. An approved operator device can SSH to the Tailscale address.
3. An unapproved tailnet device cannot SSH or reach any service.
4. A LAN peer cannot SSH or reach Aletheon/GBrain.
5. An external scanner sees no inbound Aletheon or administration port.

Never publish tailnet node keys, public IP inventories, peer identities, or full
firewall counters in a support bundle.
