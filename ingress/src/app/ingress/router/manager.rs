use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use anyhow::anyhow;
use async_trait::async_trait;
use framework::api;
use framework::incoming::cloudflare_tunnels::types::EdgeRegionLocation;
use framework::incoming::cloudflare_tunnels::types::HandleHttpTrait;
use framework::incoming::cloudflare_tunnels::types::HttpRequest;
use framework::incoming::cloudflare_tunnels::types::HttpResponse;
use framework::incoming::cloudflare_tunnels::types::HttpStream;
use framework::incoming::cloudflare_tunnels::types::TunnelAuth;
use framework::incoming::cloudflare_tunnels::Supervisor;
use tokio::io::AsyncWriteExt;
use tokio::select;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tracing::{error, info};
use utils::context::wait::Context;
use utils::context::wait::Handle;
use uuid::Uuid;

use super::RebuildConfig;

use super::types;

type Rules = Vec<types::http::Container<types::http::cloudflare_tunnel::Ingress>>;

struct RunningTunnel {
    // this is the instance of this running tunnel. Not the tunnel id
    inst_id: Uuid,
    // rules
    rules: Arc<RwLock<Arc<Rules>>>,

    auth: TunnelAuth,
}

impl RunningTunnel {
    pub fn new(inst_id: Uuid, auth: TunnelAuth) -> Self {
        Self {
            inst_id,
            auth,
            rules: Arc::new(RwLock::new(Arc::new(Vec::new()))),
        }
    }

    pub async fn serve(&self) -> Handle {
        let mut handle = Handle::new();

        let id = self.inst_id;

        let mut ctx = handle.spawn();
        let auth = self.auth.clone();
        let rules = self.rules.clone();

        tokio::spawn(async move {
            loop {
                select! {
                    _ = ctx.done() => {
                        info!("context dropped, stopping tunnel");
                        break;
                    }
                    _ = async{} => {
                        info!("starting tunnel");
                        let result = async {
                            let supervisor = Supervisor::new(id, &EdgeRegionLocation::AUTO, auth.clone(), Box::new(Arc::new(RunningTunnelHandle(rules.clone())))).await?;
                            supervisor.start(ctx.clone()).await
                        }.await;

                        if let Err(e) = result {
                            error!("Error while serving tunnel: {}", e);
                        } else {
                            info!("Tunnel {} stopped", id);
                        }

                        // TODO: wait a bit before trying again
                        select! {
                            _ = ctx.done() => {
                                info!("context dropped, stopping tunnel");
                                break;
                            }
                            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                        }
                    }
                }
            }
        });

        handle
    }

    pub async fn rebuild(&self, new_rules: Rules) {
        let mut rules = self.rules.write().await;
        *rules = Arc::new(new_rules);
    }
}

struct RunningTunnelHandle(Arc<RwLock<Arc<Rules>>>);

#[async_trait]
impl HandleHttpTrait for RunningTunnelHandle {
    async fn handle(
        &self,
        ctx: Context,
        req: HttpRequest,
        stream: Box<dyn HttpStream>,
    ) -> Result<()> {
        let mut ctx = ctx;

        select! {
            // allows for super fast shutdown, as returning from this function will close the stream
            _ = ctx.done() => { Err(anyhow!("context canceled")) },
            r = async {
                let _rules = self.0.read().await.clone();

                let response = HttpResponse {
                    headers: vec![("x-based".to_string(), "waytoobased".to_string())],
                    status: 200,
                };

                let mut stream = stream;

                let (_reader, writer) = stream.decompose(Ok(response)).await?;

                writer.write_all(format!("{:#?}", req).as_bytes()).await?;

                Ok(())
            } => {
                r
            },
        }
    }
}

pub(super) struct Manager {
    cloudflare_tunnels: Arc<Mutex<HashMap<Uuid, (RunningTunnel, Handle)>>>,
}

impl Manager {
    pub fn new() -> Self {
        Self {
            cloudflare_tunnels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn graceful(&self, ctx: Context) {
        let mp = self.cloudflare_tunnels.clone();

        let mut ctx = ctx;
        tokio::spawn(async move {
            ctx.done().await;

            let mut mp = mp.lock().await;
            for (_, (_, handle)) in mp.drain() {
                handle.cancel().await;
            }
        });
    }

    pub async fn update(&mut self, config: RebuildConfig) {
        self.update_tunnels(&config).await;
    }

    async fn update_tunnels(&mut self, config: &RebuildConfig) {
        let mut valid = HashMap::new();
        let mut mp = self.cloudflare_tunnels.lock().await;

        for (id, tunnel) in config.cloudflare_tunnels.clone() {
            if let Some(running_tunnel) = mp.get(&id) {
                running_tunnel.0.rebuild(tunnel.ingress).await;
            } else {
                let api = api::cloudflare::Client::new(
                    tunnel.account_id,
                    match tunnel.auth {
                        types::auth::Cloudflare::ApiToken { token } => {
                            api::cloudflare::Auth::ApiToken(token)
                        }
                        types::auth::Cloudflare::ApiKey { key, email } => {
                            api::cloudflare::Auth::ApiKey { key, email }
                        }
                    },
                );

                let result = api.get_tunnel_token(&tunnel.id.to_string()).await;
                if let Err(e) = result {
                    error!("Error while getting tunnel token: {}", e);
                    continue;
                }

                let auth = TunnelAuth::new(&result.unwrap());
                if let Err(e) = auth {
                    error!("Error while creating tunnel auth: {}", e);
                    continue;
                }

                let auth = auth.unwrap();

                let running_tunnel = RunningTunnel::new(id, auth);
                running_tunnel.rebuild(tunnel.ingress).await;
                let handle = running_tunnel.serve().await;

                mp.insert(id, (running_tunnel, handle));
            }

            valid.insert(id, ());
        }

        let mut to_remove = Vec::new();
        for id in mp.keys() {
            if !valid.contains_key(id) {
                to_remove.push(*id);
            }
        }

        for id in to_remove {
            if let Some((_, handle)) = mp.remove(&id) {
                handle.cancel().await;
            }
        }
    }
}
