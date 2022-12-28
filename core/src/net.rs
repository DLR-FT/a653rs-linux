//! Module for netlink operations

use std::process::Command;

use anyhow::bail;
use nix::unistd::Pid;
use regex::Regex;

pub struct VethPair {}

impl VethPair {
    pub fn new(p1: &str, p2: &str, pid: Pid) -> anyhow::Result<Self> {
        // Prevent CMD injections by only allowing [A-Za-z0-9_]
        if !p1.chars().all(|c| char::is_ascii_alphanumeric(&c))
            || !p2.chars().all(|c| char::is_ascii_alphanumeric(&c))
        {
            bail!("interface name is not well-formatted")
        }

        let cmd = Command::new("ip")
            .arg("link")
            .arg("add")
            .arg(p1)
            .arg("type")
            .arg("veth")
            .arg("peer")
            .arg(p2)
            .arg("netns")
            .arg(pid.to_string())
            .output()?;

        if !cmd.status.success() {
            bail!("{}", String::from_utf8(cmd.stderr)?)
        }

        anyhow::Ok(Self {})
    }

    // A Drop trait is not necessary, because the veth interface pair will
    // automatically be deleted, if one side of the pair gets deleted, which
    // is the case when the child net namespace goes out of scope.
}

/// Returns all interfaces available in the current namespace
///
/// The '@' part of an interface will not be included.
pub fn get_interfaces() -> anyhow::Result<Vec<String>> {
    let cmd = Command::new("sh").arg("-c").arg("ip address").output()?;
    if !cmd.status.success() {
        bail!("ip-address(8) failed")
    }

    let str = String::from_utf8(cmd.stdout)?;
    let re = Regex::new(r"^\d+: ([\w\d]+)")?;
    let mut result: Vec<String> = Vec::new();

    for line in str.lines() {
        for cap in re.captures_iter(line) {
            result.push(cap[1].to_string());
        }
    }

    anyhow::Ok(result)
}
