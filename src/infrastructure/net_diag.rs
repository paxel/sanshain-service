//! Network diagnosis for a host the service could not reach, run from inside
//! the container so the answer describes the pod's own view of the network.
//!
//! The admin's "Test Connection" appends these lines when the directory was
//! unreachable: what the hostname resolves to, which resolvers answered, and
//! for every address whether an ICMP echo, the kernel's route lookup and a
//! TCP connect to the port succeed. Each probe is bounded by
//! [`PROBE_TIMEOUT`], the whole diagnosis by [`TOTAL_TIMEOUT`].
//!
//! The ICMP echo uses an unprivileged datagram ICMP socket. The kernel allows
//! those for the groups in `net.ipv4.ping_group_range`; Docker opens that
//! range by default, Kubernetes does not unless the pod sets the sysctl, and
//! the line then says so instead of pretending the host is silent.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};

/// How long a single probe (echo, TCP connect, lookup) may take.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long the whole diagnosis may take; the probes run concurrently, so
/// this is a guard rather than a sum.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the resolvers are read from.
const RESOLV_CONF: &str = "/etc/resolv.conf";

/// Diagnose why `host:port` could not be reached. One line per finding, in
/// the order an admin reads them: name resolution, resolvers, one line per
/// address. Never fails: a probe that cannot run says so in its line.
pub async fn diagnose(host: &str, port: u16) -> Vec<String> {
    let mut lines = Vec::new();
    let all = tokio::time::timeout(TOTAL_TIMEOUT, async {
        let addrs = resolve(host, port, &mut lines).await;
        lines.push(resolvers_line(
            &std::fs::read_to_string(RESOLV_CONF).unwrap_or_default(),
        ));
        let probes: Vec<_> = addrs
            .iter()
            .map(|addr| tokio::spawn(probe_address(*addr)))
            .collect();
        for probe in probes {
            match probe.await {
                Ok(line) => lines.push(line),
                Err(e) => lines.push(format!("probe failed: {e}")),
            }
        }
    })
    .await;
    if all.is_err() {
        lines.push(format!(
            "diagnosis stopped after {} s",
            TOTAL_TIMEOUT.as_secs_f64()
        ));
    }
    lines
}

/// Resolve the host, appending the DNS line, and hand back the addresses to
/// probe. A literal address needs no lookup and says so.
async fn resolve(host: &str, port: u16, lines: &mut Vec<String>) -> Vec<SocketAddr> {
    if let Ok(ip) = host
        .trim_matches(|c| c == '[' || c == ']')
        .parse::<IpAddr>()
    {
        lines.push(format!("DNS: {host} is a literal address, no lookup"));
        return vec![SocketAddr::new(ip, port)];
    }
    let started = Instant::now();
    let lookup = tokio::time::timeout(PROBE_TIMEOUT, tokio::net::lookup_host((host, port))).await;
    let elapsed = millis(started);
    match lookup {
        Ok(Ok(addrs)) => {
            let addrs: Vec<SocketAddr> = addrs.collect();
            if addrs.is_empty() {
                lines.push(format!("DNS: {host} resolved to no address ({elapsed})"));
            } else {
                let list: Vec<String> = addrs.iter().map(|a| a.ip().to_string()).collect();
                lines.push(format!(
                    "DNS: {host} resolves to {} ({elapsed})",
                    list.join(", ")
                ));
            }
            addrs
        }
        Ok(Err(e)) => {
            lines.push(format!("DNS: {host} did not resolve: {e} ({elapsed})"));
            Vec::new()
        }
        Err(_) => {
            lines.push(format!(
                "DNS: no answer for {host} within {} s",
                PROBE_TIMEOUT.as_secs_f64()
            ));
            Vec::new()
        }
    }
}

/// The resolvers line from the text of `/etc/resolv.conf`.
fn resolvers_line(resolv_conf: &str) -> String {
    let mut nameservers = Vec::new();
    let mut search = Vec::new();
    for line in resolv_conf.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("nameserver") => nameservers.extend(words.next().map(str::to_string)),
            Some("search") => search.extend(words.map(str::to_string)),
            _ => {}
        }
    }
    if nameservers.is_empty() {
        return "Resolvers: none found in /etc/resolv.conf".to_string();
    }
    let mut line = format!("Resolvers: {}", nameservers.join(", "));
    if !search.is_empty() {
        line.push_str(&format!(" (search {})", search.join(", ")));
    }
    line
}

/// The three probes of one address, concurrently, on one line.
async fn probe_address(addr: SocketAddr) -> String {
    let (ping, tcp) = tokio::join!(ping(addr.ip()), tcp_probe(addr));
    let route = route_lookup(addr.ip());
    format!(
        "{}: ping {ping}; route {route}; tcp/{} {tcp}",
        addr.ip(),
        addr.port()
    )
}

/// Which local address the kernel would send from, found by connecting a UDP
/// socket (no packet leaves). "no route" is the kernel's own verdict.
pub fn route_lookup(ip: IpAddr) -> String {
    let unspecified = match ip {
        IpAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        IpAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
    };
    let socket = match std::net::UdpSocket::bind(unspecified) {
        Ok(socket) => socket,
        Err(e) => return format!("lookup failed: {e}"),
    };
    // Any port does: the lookup depends on the destination address only.
    match socket.connect(SocketAddr::new(ip, 9)) {
        Ok(()) => match socket.local_addr() {
            Ok(local) => format!("via local address {}", local.ip()),
            Err(e) => format!("lookup failed: {e}"),
        },
        Err(e) if is_unreachable(&e) => "no route from this container".to_string(),
        Err(e) => format!("lookup failed: {e}"),
    }
}

/// One TCP connect to the port.
pub async fn tcp_probe(addr: SocketAddr) -> String {
    let started = Instant::now();
    match tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(_)) => format!("accepted in {}", millis(started)),
        Ok(Err(e)) if e.kind() == io::ErrorKind::ConnectionRefused => "refused".to_string(),
        Ok(Err(e)) if is_unreachable(&e) => "no route".to_string(),
        Ok(Err(e)) => e.to_string(),
        Err(_) => format!("no answer within {} s", PROBE_TIMEOUT.as_secs_f64()),
    }
}

/// One ICMP echo request through an unprivileged datagram ICMP socket.
pub async fn ping(ip: IpAddr) -> String {
    let socket = match icmp_socket(ip) {
        Ok(socket) => socket,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            return "not permitted in this container (net.ipv4.ping_group_range)".to_string();
        }
        Err(e) if e.raw_os_error() == Some(libc_eprotonosupport()) => {
            return "not supported on this host".to_string();
        }
        Err(e) => return format!("socket failed: {e}"),
    };
    let started = Instant::now();
    match tokio::time::timeout(PROBE_TIMEOUT, echo(&socket, ip)).await {
        Ok(Ok(())) => format!("answered in {}", millis(started)),
        Ok(Err(e)) if is_unreachable(&e) => "no route".to_string(),
        Ok(Err(e)) => e.to_string(),
        Err(_) => format!("no answer within {} s", PROBE_TIMEOUT.as_secs_f64()),
    }
}

/// A non-blocking datagram ICMP socket of the address's family, registered
/// with tokio. The kernel picks and checks the echo identifier for these.
fn icmp_socket(ip: IpAddr) -> io::Result<tokio::net::UdpSocket> {
    let (domain, protocol) = match ip {
        IpAddr::V4(_) => (Domain::IPV4, Protocol::ICMPV4),
        IpAddr::V6(_) => (Domain::IPV6, Protocol::ICMPV6),
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(protocol))?;
    socket.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(std::net::UdpSocket::from(socket))
}

/// Send one echo request and wait for the echo reply.
async fn echo(socket: &tokio::net::UdpSocket, ip: IpAddr) -> io::Result<()> {
    let request = echo_request(ip);
    socket.send_to(&request, SocketAddr::new(ip, 0)).await?;
    let mut buf = [0u8; 1500];
    loop {
        let (len, _) = socket.recv_from(&mut buf).await?;
        if is_echo_reply(ip, buf.get(..len).unwrap_or(&[])) {
            return Ok(());
        }
    }
}

/// An echo request with an eight-byte payload. The identifier is filled in
/// by the kernel for datagram ICMP sockets; the sequence is 1.
fn echo_request(ip: IpAddr) -> [u8; 16] {
    let kind = match ip {
        IpAddr::V4(_) => 8,
        IpAddr::V6(_) => 128,
    };
    let mut packet = [0u8; 16];
    packet[0] = kind;
    packet[7] = 1;
    packet[8..].copy_from_slice(b"sanshain");
    if ip.is_ipv4() {
        let checksum = internet_checksum(&packet);
        packet[2..4].copy_from_slice(&checksum.to_be_bytes());
    }
    // ICMPv6 carries a pseudo-header checksum the kernel always computes.
    packet
}

/// The one's-complement checksum ICMPv4 carries.
fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in data.chunks(2) {
        let word = match chunk {
            [a, b] => u16::from_be_bytes([*a, *b]),
            [a] => u16::from_be_bytes([*a, 0]),
            _ => 0,
        };
        sum = sum.saturating_add(u32::from(word));
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff).saturating_add(sum >> 16);
    }
    !(sum as u16)
}

/// Whether a received datagram is an echo reply. Some kernels hand the IPv4
/// header back in front of the ICMP header; it is skipped when present.
fn is_echo_reply(ip: IpAddr, datagram: &[u8]) -> bool {
    let icmp = match (ip, datagram.first()) {
        (IpAddr::V4(_), Some(&first)) if first >> 4 == 4 => {
            let header_len = usize::from(first & 0x0f).saturating_mul(4);
            datagram.get(header_len..).unwrap_or(&[])
        }
        _ => datagram,
    };
    let expected = match ip {
        IpAddr::V4(_) => 0,
        IpAddr::V6(_) => 129,
    };
    icmp.first() == Some(&expected)
}

/// The two "no way there" answers of the kernel.
fn is_unreachable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable
    )
}

/// `EPROTONOSUPPORT`, the answer of a kernel without datagram ICMP sockets.
fn libc_eprotonosupport() -> i32 {
    93
}

fn millis(since: Instant) -> String {
    format!("{:.1} ms", since.elapsed().as_secs_f64() * 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolvers_line_lists_nameservers_and_search_domains() {
        let text = "# comment\nnameserver 10.0.0.2\nnameserver fd00::53\nsearch svc.cluster.local cluster.local\noptions ndots:5\n";
        assert_eq!(
            resolvers_line(text),
            "Resolvers: 10.0.0.2, fd00::53 (search svc.cluster.local, cluster.local)"
        );
    }

    #[test]
    fn resolvers_line_without_nameservers_says_so() {
        assert_eq!(
            resolvers_line("search example\n"),
            "Resolvers: none found in /etc/resolv.conf"
        );
        assert_eq!(
            resolvers_line(""),
            "Resolvers: none found in /etc/resolv.conf"
        );
    }

    #[test]
    fn the_echo_request_carries_a_valid_checksum() {
        let packet = echo_request("127.0.0.1".parse().expect("literal"));
        assert_eq!(packet[0], 8, "echo request");
        assert_eq!(
            internet_checksum(&packet),
            0,
            "checksum over a checksummed packet is zero"
        );
        let v6 = echo_request("::1".parse().expect("literal"));
        assert_eq!(v6[0], 128);
        assert_eq!(
            &v6[2..4],
            &[0, 0],
            "the kernel computes the ICMPv6 checksum"
        );
    }

    #[test]
    fn echo_replies_are_recognised_with_and_without_an_ip_header() {
        let v4: IpAddr = "127.0.0.1".parse().expect("literal");
        let mut with_header = vec![0x45u8; 1];
        with_header.extend([0u8; 19]);
        with_header.extend([0u8, 0, 0, 0, 0, 0, 0, 1]);
        assert!(is_echo_reply(v4, &with_header));
        assert!(is_echo_reply(v4, &[0, 0, 0, 0, 0, 0, 0, 1]));
        assert!(
            !is_echo_reply(v4, &[8, 0, 0, 0, 0, 0, 0, 1]),
            "an echo request is not a reply"
        );
        let v6: IpAddr = "::1".parse().expect("literal");
        assert!(is_echo_reply(v6, &[129, 0]));
        assert!(!is_echo_reply(v6, &[128, 0]));
        assert!(!is_echo_reply(v4, &[]));
    }

    #[test]
    fn route_lookup_names_the_local_address_for_loopback() {
        assert_eq!(
            route_lookup("127.0.0.1".parse().expect("literal")),
            "via local address 127.0.0.1"
        );
    }

    #[test]
    fn route_lookup_reports_no_route_for_an_unroutable_address() {
        // 192.0.2.0/24 is TEST-NET-1, never routed; a host with a default
        // route still has a route to it, so only a sandbox without one shows
        // "no route". Either answer is a correct lookup; what must not happen
        // is a failure.
        let answer = route_lookup("192.0.2.1".parse().expect("literal"));
        assert!(
            answer.starts_with("via local address ") || answer == "no route from this container",
            "{answer}"
        );
    }

    #[tokio::test]
    async fn tcp_probe_tells_an_open_port_from_a_closed_one() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let open = listener.local_addr().expect("addr");
        assert!(tcp_probe(open).await.starts_with("accepted in "));
        assert_eq!(
            tcp_probe("127.0.0.1:1".parse().expect("literal")).await,
            "refused"
        );
    }

    #[tokio::test]
    async fn ping_answers_for_loopback_or_names_the_sysctl() {
        let answer = ping("127.0.0.1".parse().expect("literal")).await;
        assert!(
            answer.starts_with("answered in ")
                || answer == "not permitted in this container (net.ipv4.ping_group_range)",
            "{answer}"
        );
    }

    #[tokio::test]
    async fn diagnose_a_literal_address_gives_dns_resolvers_and_one_probe_line() {
        let lines = diagnose("127.0.0.1", 1).await;
        assert_eq!(lines[0], "DNS: 127.0.0.1 is a literal address, no lookup");
        assert!(lines[1].starts_with("Resolvers: "), "{}", lines[1]);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[2].starts_with("127.0.0.1: ping "), "{}", lines[2]);
        assert!(
            lines[2].contains("; route via local address 127.0.0.1; tcp/1 refused"),
            "{}",
            lines[2]
        );
    }

    #[tokio::test]
    async fn diagnose_a_name_that_does_not_resolve_has_no_probe_lines() {
        let lines = diagnose("no-such-host.invalid", 389).await;
        assert!(
            lines[0].starts_with("DNS: no-such-host.invalid did not resolve: "),
            "{}",
            lines[0]
        );
        assert_eq!(lines.len(), 2, "{lines:?}");
    }
}
