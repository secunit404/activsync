import ipaddress

from activsync import net_gateways

IPV4_ROUTES = """\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eth0\t00000000\t010012AC\t0003\t0\t0\t0\t00000000\t0\t0\t0
eth0\t000012AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
"""

IPV6_ROUTES = """\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 20010db8000000000000000000000001 00000400 00000001 00000000 00000003 eth0
20010db8000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001 eth0
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200 lo
"""


def test_ipv4_default_gateway_is_decoded_from_little_endian_hex():
    assert net_gateways.parse_ipv4_gateways(IPV4_ROUTES) == (
        ipaddress.IPv4Address("172.18.0.1"),
    )


def test_ipv4_on_link_default_route_has_no_gateway():
    text = "eth0\t00000000\t00000000\t0001\t0\t0\t0\t00000000\t0\t0\t0\n"

    assert net_gateways.parse_ipv4_gateways(text) == ()


def test_ipv6_default_route_next_hop_is_the_gateway():
    assert net_gateways.parse_ipv6_gateways(IPV6_ROUTES) == (
        ipaddress.IPv6Address("2001:db8::1"),
    )


def test_ipv6_default_route_with_an_all_zero_next_hop_is_ignored():
    assert net_gateways.parse_ipv6_gateways(IPV6_ROUTES.splitlines()[2]) == ()


def test_malformed_rows_are_skipped():
    assert net_gateways.parse_ipv4_gateways("eth0 00000000 zzzzzzzz\neth0 00000000\n\n") == ()
    assert net_gateways.parse_ipv6_gateways("0" * 32 + " 00 " + "0" * 32 + " 00 nothex\n") == ()


def test_both_tables_are_read(tmp_path):
    v4 = tmp_path / "route"
    v6 = tmp_path / "ipv6_route"
    v4.write_text(IPV4_ROUTES)
    v6.write_text(IPV6_ROUTES)

    assert net_gateways.read_default_gateways(v4, v6) == (
        ipaddress.IPv4Address("172.18.0.1"),
        ipaddress.IPv6Address("2001:db8::1"),
    )


def test_unreadable_tables_mean_no_gateways(tmp_path):
    missing = tmp_path / "absent"
    undecodable = tmp_path / "binary"
    undecodable.write_bytes(b"\xff\xfe\x00")

    assert net_gateways.read_default_gateways(missing, missing) == ()
    assert net_gateways.read_default_gateways(undecodable, tmp_path) == ()
