use config::{AuthConfig, Config, HostConfig, LocationConfig};
use data::{Data, HostData};
use ips::IpBlock;

use std::error;
use std::path;
use std::io::{self, Write};
use std::net::TcpStream;
use std::time::Duration;

use crossbeam;
use rayon::prelude::*;
use serde_json;
use ssh2;
use time;

fn fetch_clean_host_data(
    host: &HostConfig,
    default: Option<&AuthConfig>,
    locations: &[LocationConfig],
) -> Option<HostData> {
    match fetch_host_data(host, default, locations) {
        Ok(mut result) => {
            result.disks.retain(|data| {
                host.ignored_disks
                    .as_ref()
                    .map(|disks| {
                        !disks.contains(&data.name) &&
                            !disks.contains(&data.mountpoint)
                    })
                    .unwrap_or(true)
            });
            Some(result)
        }
        Err(e) => {
            println!("Error with {}: {:?}", host.name, e);
            None
        }
    }
}

fn fill_result(result: &mut Vec<Option<HostData>>, config: &Config) {
    let default = config.default.as_ref();
    let locations = &config.locations;
    let iter = result.iter_mut().zip(config.hosts.iter());
    crossbeam::scope(|scope| for (r, host) in iter {
        scope.spawn(move || {
            *r = fetch_clean_host_data(host, default, locations);
        });
    });
}

pub fn fetch_data(config: &Config) -> Data {
    // Fetch each host in parallel
    let mut result: Vec<_> = config.hosts.iter().map(|_| None).collect();
    fill_result(&mut result, config);
    let mut result: Vec<_> = result.into_iter().filter_map(|r| r).collect();

    let empty = String::new();
    result.sort_by(|a, b| {
        a.location
            .as_ref()
            .unwrap_or(&empty)
            .cmp(b.location.as_ref().unwrap_or(&empty))
    });

    let now = format!("{}", time::now().rfc3339());
    Data {
        hosts: result,
        update_time: now,
    }
}

fn authenticate(
    sess: &mut ssh2::Session,
    host: &HostConfig,
    default: Option<&AuthConfig>,
) -> Result<(), ssh2::Error> {
    // Do we have an authentication? Or do we have a default one?
    if let Some(auth) = host.auth.as_ref().or(default) {
        if let Some(ref password) = auth.password {
            // Maybe we log in with a password?
            sess.userauth_password(&auth.login, password)?;
        } else if let Some(ref keypair) = auth.keypair {
            // Or maybe with an identity file?
            sess.userauth_pubkey_file(
                &auth.login,
                None,
                path::Path::new(keypair),
                None,
            )?;
        }
    }
    Ok(())
}

type BoxedError = Box<error::Error + Send + Sync>;

fn connect(
    host: &HostConfig,
    default: Option<&AuthConfig>,
) -> Result<(TcpStream, ssh2::Session), BoxedError> {
    // TODO: Don't panic on error
    let tcp = TcpStream::connect((&*host.address, 22))?;
    tcp.set_read_timeout(Some(Duration::from_secs(15)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(15)))?;

    // An error here means something very wrong is going on.
    let mut sess = ssh2::Session::new().ok_or_else(|| {
        io::Error::new(io::ErrorKind::Other, "Could not create ssh session")
    })?;
    // 15,000 ms = 15s
    sess.set_timeout(15_000);
    sess.handshake(&tcp)?;
    authenticate(&mut sess, host, default)?;

    Ok((tcp, sess))
}


fn fetch_host_data(
    host: &HostConfig,
    default: Option<&AuthConfig>,
    locations: &[LocationConfig],
) -> Result<HostData, Box<error::Error + Send + Sync>> {
    // `tcp` needs to survive the scope,
    // because on drop it closes the connection.
    // But we're not using it, so an underscore
    // will avoid `unused` warnings.
    let (_tcp, sess) = connect(host, default)?;

    let mut channel = sess.channel_session()?;
    channel.exec(&format!("./fetch.py {}", host.iface))?;
    // A JSON error here means the script went mad.
    // ... or just a connection issue maybe?
    let mut result: HostData = serde_json::from_reader(channel)?;
    let location = result
        .network
        .as_ref()
        .and_then(|n| n.ip.as_ref())
        .and_then(|ip| find_location(ip, locations));

    result.location = host.location.clone().or(location);

    Ok(result)
}

fn find_location(ip: &str, locations: &[LocationConfig]) -> Option<String> {
    locations
        .iter()
        .find(|l| match_ip(ip, &l.ips))
        .map(|l| l.name.clone())
}

fn match_ip(ip: &str, mask: &str) -> bool {
    IpBlock::new(mask).matches(ip)
}

fn prepare_host(
    host: &HostConfig,
    default: Option<&AuthConfig>,
) -> Result<(), Box<error::Error + Send + Sync>> {
    // Directly include the script in the executable
    let script_data = include_str!("../data/fetch.py");

    // `tcp` needs to survive the scope, because on drop it closes the connection.
    let (_tcp, sess) = connect(host, default)?;
    let mut remote_file = sess.scp_send(
        path::Path::new("fetch.py"),
        0o755,
        script_data.len() as u64,
        None,
    )?;
    remote_file.write_all(script_data.as_bytes())?;
    Ok(())
}

pub fn prepare_hosts(
    config: &Config,
) -> Vec<Option<Box<error::Error + Send + Sync>>> {
    let mut result = Vec::new();
    // Prepare each host in parallel
    config
        .hosts
        .par_iter()
        .map(|host| prepare_host(host, config.default.as_ref()).err())
        .collect_into(&mut result);
    result
}
