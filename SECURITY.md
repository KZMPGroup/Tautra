# Security policy

Tautra controls keyboard and mouse input and moves clipboard data and files between devices, so security reports are taken seriously.

## Reporting a vulnerability

Please do not open a public issue for a security problem. Use GitHub's private reporting instead: go to the repository's **Security** tab and choose **Report a vulnerability**.

Include the affected version or commit, the steps to reproduce, and what an attacker could do.

## Scope

In scope:

- Connecting to, or injecting input into, a device without pairing.
- Bypassing certificate pinning or the pairing PIN.
- Reading clipboard data or files that were not shared.
- Getting a malicious update accepted.
- Gaining privileges through the Tautra Windows service.

Out of scope: attacks that need administrator access, or physical access to an already-unlocked paired device.
