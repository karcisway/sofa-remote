//! Wake-on-LAN data for the phone: the MAC address and broadcast address of this PC's network
//! adapters. A sleeping PC can't answer, so the phone remembers them while the PC is awake and
//! sends the "magic packet" itself later (the Android app does, a web page can't send UDP).

use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
use serde_json::{Value, json};
use std::net::{IpAddr, Ipv4Addr};

#[derive(Debug, PartialEq, Eq)]
struct Adapter {
    mac: String,
    ip: Ipv4Addr,
    netmask: Ipv4Addr,
    broadcast: Ipv4Addr,
}

fn adapters() -> Vec<Adapter> {
    let Ok(list) = NetworkInterface::show() else { return Vec::new() };
    let mut out = Vec::new();
    for iface in list {
        let Some(mac) = iface.mac_addr.filter(|m| m.len() == 17 && m != "00:00:00:00:00:00") else { continue };
        for addr in iface.addr {
            let Addr::V4(v4) = addr else { continue };
            if v4.ip.is_loopback() || v4.ip.is_link_local() || v4.ip.is_unspecified() {
                continue;
            }
            let netmask = v4.netmask.unwrap_or(Ipv4Addr::new(255, 255, 255, 0));
            let broadcast = v4.broadcast.unwrap_or_else(|| Ipv4Addr::from(u32::from(v4.ip) | !u32::from(netmask)));
            out.push(Adapter { mac: mac.to_lowercase().replace('-', ":"), ip: v4.ip, netmask, broadcast });
        }
    }
    out
}

/// The adapters on the same network as `peer`; all of them when that can't be told.
fn pick(all: Vec<Adapter>, peer: IpAddr) -> Vec<Adapter> {
    let IpAddr::V4(peer) = peer else { return all };
    let same_net = |a: &Adapter| {
        let mask = u32::from(a.netmask);
        u32::from(a.ip) & mask == u32::from(peer) & mask
    };
    if all.iter().any(same_net) { all.into_iter().filter(same_net).collect() } else { all }
}

/// `[{"mac": "aa:bb:cc:dd:ee:ff", "bcast": "192.168.1.255"}]`, sent to a phone in the hello.
pub fn targets(peer: IpAddr) -> Vec<Value> {
    pick(adapters(), peer).into_iter().map(|a| json!({ "mac": a.mac, "bcast": a.broadcast.to_string() })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(mac: &str, ip: [u8; 4], bcast: [u8; 4]) -> Adapter {
        Adapter { mac: mac.into(), ip: ip.into(), netmask: Ipv4Addr::new(255, 255, 255, 0), broadcast: bcast.into() }
    }

    #[test]
    fn keeps_the_adapter_on_the_phones_network() {
        let all = vec![adapter("a", [10, 0, 0, 5], [10, 0, 0, 255]), adapter("b", [192, 168, 1, 20], [192, 168, 1, 255])];
        let picked = pick(all, "192.168.1.77".parse().unwrap());
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].mac, "b");
    }

    #[test]
    fn keeps_all_when_none_match() {
        let all = vec![adapter("a", [10, 0, 0, 5], [10, 0, 0, 255]), adapter("b", [192, 168, 1, 20], [192, 168, 1, 255])];
        assert_eq!(pick(all, "172.16.0.9".parse().unwrap()).len(), 2);
    }
}
