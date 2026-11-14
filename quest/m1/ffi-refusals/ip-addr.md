# [S] IpAddr crosses by family

## Goal

`std::net::IpAddr`, `Ipv4Addr`, and `Ipv6Addr` export in every binding, as `SocketAddr`
already does: its IP's bytes and whether it is IPv6, with no port. moq-tokio's
`quic::demux::Datagram::local` is the one item at moq's `main` (`a11a584`, measured on
2026-11-14 in [#802](https://github.com/kixel-ai/rust2/pull/802)) refused for it, and the
census lists no `IpAddr` refusal there.

## Plan

Decided with the maintainer on 2026-11-13, in
[moq-excluded](https://github.com/kixel-ai/rust2/pull/802)'s Plan: ✅ `IpAddr` crosses as
`SocketAddr` already does, by family. #802 split it out, since it is a new IR type
through the glue and all seven generators, apart from that quest's collector rules.

- A new `<prefix>_ip_addr` C struct of 16 bytes and an `is_v6` flag, `SocketAddr`'s
  without the port, and each binding's own address type where it is exact, as each maps
  `SocketAddr` today (Kotlin `InetAddress`, Python `ipaddress`, Swift, Go `netip.Addr`,
  Dart `InternetAddress`, C++). Recommended: reuse each binding's `SocketAddr` mapping of
  the IP for the address and drop the port, so one family check covers both.
- `Ipv4Addr` and `Ipv6Addr` cross as that struct too; one of the other family passed in
  aborts, as `SocketAddrV4` does.
- Prove it in the api fixture's transcript in all seven languages, both families, in and
  out.

Public API: the generated API of each item that newly exports.
