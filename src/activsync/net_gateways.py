"""The container's default gateways, read from the kernel routing tables."""

from __future__ import annotations

import ipaddress
from collections.abc import Callable
from pathlib import Path

IPAddress = ipaddress.IPv4Address | ipaddress.IPv6Address

IPV4_ROUTES = Path("/proc/net/route")
IPV6_ROUTES = Path("/proc/net/ipv6_route")

_IPV4_DEFAULT_DESTINATION = "00000000"
_IPV6_DEFAULT_DESTINATION = "0" * 32


def parse_ipv4_gateways(text: str) -> tuple[ipaddress.IPv4Address, ...]:
    """Gateways of default routes in /proc/net/route, where the address is a
    little-endian hex word."""
    gateways = []
    for fields in (line.split() for line in text.splitlines()):
        if len(fields) < 3 or fields[1] != _IPV4_DEFAULT_DESTINATION:
            continue
        try:
            gateway = ipaddress.IPv4Address(int(fields[2], 16).to_bytes(4, "little"))
        except (ValueError, OverflowError):
            continue
        if not gateway.is_unspecified:
            gateways.append(gateway)
    return tuple(gateways)


def parse_ipv6_gateways(text: str) -> tuple[ipaddress.IPv6Address, ...]:
    """Next hops of default routes in /proc/net/ipv6_route, whose columns are
    destination, its prefix length, source, its prefix length, then next hop."""
    gateways = []
    for fields in (line.split() for line in text.splitlines()):
        if (
            len(fields) < 5
            or fields[0] != _IPV6_DEFAULT_DESTINATION
            or fields[1] != "00"
        ):
            continue
        try:
            gateway = ipaddress.IPv6Address(bytes.fromhex(fields[4]))
        except ValueError:
            continue
        if not gateway.is_unspecified:
            gateways.append(gateway)
    return tuple(gateways)


def _read(path: Path, parse: Callable[[str], tuple[IPAddress, ...]]) -> tuple[IPAddress, ...]:
    try:
        return parse(path.read_text())
    except (OSError, ValueError):
        return ()


def read_default_gateways(
    ipv4_routes: Path = IPV4_ROUTES, ipv6_routes: Path = IPV6_ROUTES
) -> tuple[IPAddress, ...]:
    """Empty where the tables do not exist, such as a macOS dev machine."""
    return (*_read(ipv4_routes, parse_ipv4_gateways), *_read(ipv6_routes, parse_ipv6_gateways))
