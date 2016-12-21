use config::{Config, HostConfig};
use server;

use std::sync::Arc;
use hyper;
use serde_json;
use reroute;
use tera::Tera;

// We'll use a server::Server to actually process anything.
// Here, we just set up the http handlers to redirect queries.
// We do the translation from request to json to actual types.
pub fn serve<F>(config: Config, config_sync: F)
    where F: 'static + Send + Sync + Fn(&Config)
{
    let port = config.http.as_ref().map_or(8080, |http| http.port);
    let config_sync_ = Arc::new(config_sync);

    // This is an Arc
    let server = server::Server::new(config);

    let mut builder = reroute::RouterBuilder::new();

    let mut tera = Tera::default();
    tera.add_template("index.html", include_str!("../data/templates/index.html")).unwrap();
    tera.add_template("style.css", include_str!("../data/templates/style.css")).unwrap();
    let server_ = server.clone();
    builder.get("^/$", move |_, resp, _| {
        // Return plain HTML
        let data = server_.latest_data();
        let content = tera.value_render("index.html", &*data).unwrap();
        resp.send(content.as_bytes()).ok();
    });

    let server_ = server.clone();
    builder.get("^/status$", move |_, resp, _| {
        let data = server_.latest_data();
        resp.send(&serde_json::to_vec(&*data).unwrap()).ok();
    });

    let server_ = server.clone();
    builder.get("^/config$", move |_, resp, _| {
        let config = server_.current_conf();
        resp.send(&serde_json::to_vec(&*config).unwrap()).ok();
    });

    // All those are just json API

    let server_ = server.clone();
    builder.post("^/refresh$", move |_, resp, _| {
        server_.refresh();
        resp.send(b"refreshed").ok();
    });

    let server_ = server.clone();
    let config_sync = config_sync_.clone();
    builder.post(r"^/add/(.+)$", move |_, resp, captures| {
        // Add a host
        // Reads hostname -> creates a bare host, then start editing it.
        let name = &captures.unwrap()[1];
        // TODO: Check for valid hostname?
        // Meh~
        // We don't even set a authentication setting here.
        match server_.with_conf(|conf| {
            // Look for existing host
            if let Some(_) = conf.hosts
                .iter()
                .find(|host| &host.address == name || &host.name == name) {
                return Err(format!("Host {} already exists", name));
            }

            conf.hosts.push(HostConfig {
                name: name.to_string(),
                address: name.to_string(),
                iface: "em1".to_string(),
                ..HostConfig::default()
            });
            config_sync(&conf);
            Ok(())
        }) {
            Ok(_) => resp.send(b"added").ok(),
            Err(msg) => resp.send(msg.as_bytes()).ok(),
        };
    });

    let server_ = server.clone();
    let config_sync = config_sync_.clone();
    builder.post("^/edit/(.+)$", move |req, resp, captures| {
        // Edit a host
        let hostname = &captures.unwrap()[1];
        // Look for existing host
        match server_.with_conf(|conf| {
            match conf.hosts
                .iter_mut()
                .find(|host| &host.address == hostname) {
                Some(host) => {
                    // Do the actual edit.
                    // Maybe directly a serialized HostConfig?
                    // That we'd read from the request... cool!
                    match serde_json::from_reader(req) {
                        Ok(conf) => {
                            *host = conf;
                        }
                        Err(e) => {
                            return Err(format!("Invalid body: {:?}", e));
                        }
                    }
                }
                None => {
                    // Return an error: host not found
                    return Err("Host not found".to_string());
                }
            }
            config_sync(&conf);
            Ok(())
        }) {
            Ok(_) => resp.send(b"ok").ok(),
            Err(msg) => resp.send(msg.as_bytes()).ok(),
        };
    });

    let server_ = server.clone();
    builder.post("^/stop$", move |_, resp, _| {
        // Edit a host
        server_.stop();
        resp.send(b"stopped").ok();
    });

    let router = builder.finalize().unwrap();

    println!("Now listening on port :{}", port);
    hyper::Server::http(("localhost", port)).unwrap().handle(router).unwrap();
}
