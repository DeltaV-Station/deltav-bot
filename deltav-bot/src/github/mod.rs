use std::cell::LazyCell;

use axum::{
    Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use jsonwebtoken::EncodingKey;
use octocrab::{
    Octocrab, OctocrabBuilder,
    models::{
        AppId, RepositoryId,
        webhook_events::{
            WebhookEvent, WebhookEventPayload,
            payload::{IssueCommentWebhookEventAction, PullRequestWebhookEventAction},
        },
    },
};
use regex::Regex;
use serde::Deserialize;
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tracing::{error, info, warn};

pub struct GhAppConfig {
    pub id: AppId,
    pub key: EncodingKey,
    pub repo_owner: String,
    pub repo_name: String,
}

// TODO: Stop using repo_owner and repo_name in calls and use repo with by_id functions instead.
pub struct GitHub {
    pub octo_app: Octocrab,
    pub octo_install: Octocrab,
    pub repo_owner: String,
    pub repo_name: String,
    pub repo: RepositoryId,
}

pub struct WebhookServer {
    pub thread: JoinHandle<()>,
    pub receiver: mpsc::Receiver<GitHubMessage>,
}

#[derive(Clone, Debug)]
pub enum GitHubMessage {
    PrOpened {
        pr_id: u64,
        pr_title: String,
        pr_body: Option<String>,
        opened_by: String,
    },
    PrClosed {
        pr_id: u64,
        closed_by: String,
    },
    PrMerged {
        pr_id: u64,
        merged_by: String,
    },
    AuthorCommented {
        issue_id: u64,
        username: String,
        comment: String,
    },
}

#[derive(Clone, Debug)]
struct ServerState {
    sender: mpsc::Sender<GitHubMessage>,
    webhook_secret: String,
}

#[derive(Deserialize, Debug)]
struct WebhookQuery {
    key: String,
}

const HTML_COMMENT_REGEX: LazyCell<Regex> = LazyCell::new(|| Regex::new("<!--(.*?)-->").unwrap());

async fn on_webhook_request(
    State(state): State<ServerState>,
    query: Query<WebhookQuery>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    if query.key != state.webhook_secret {
        return StatusCode::UNAUTHORIZED;
    }

    let Some(event_header) = headers.get("X-GitHub-Event").and_then(|x| x.to_str().ok()) else {
        warn!("Authorized request missing header!!");
        return StatusCode::BAD_REQUEST;
    };

    let body: String = body.chars().filter(|c| !c.is_control()).collect(); // remove control characters, serde_json will be angry if there are any

    let event = match WebhookEvent::try_from_header_and_body(event_header, &body) {
        Ok(x) => x,
        Err(e) => {
            error!("Failed to parse webhook event: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };

    let sender_login = event
        .sender
        .and_then(|x| Some(x.login))
        .unwrap_or("UNKNOWN_USER".into());

    use WebhookEventPayload::*;
    match event.specific {
        PullRequest(p) => {
            use PullRequestWebhookEventAction::*;
            match p.action {
                Closed => {
                    if p.pull_request.merged.is_none() {
                        warn!(
                            "Received PullRequest Close event with 'merged' unset, assuming closed without merging."
                        );
                    }

                    if p.pull_request.merged.unwrap_or_default() {
                        state
                            .sender
                            .send(GitHubMessage::PrMerged {
                                pr_id: p.number,
                                merged_by: sender_login,
                            })
                            .await
                            .expect("Failed to send PrMerged message");
                    } else {
                        state
                            .sender
                            .send(GitHubMessage::PrClosed {
                                pr_id: p.number,
                                closed_by: sender_login,
                            })
                            .await
                            .expect("Failed to send PrClosed message");
                    }
                }

                Opened | Reopened | ReadyForReview => {
                    if let Some(draft) = p.pull_request.draft
                        && draft
                    {
                        info!(
                            "Received PullRequest Open event for draft pr {}. Ignoring.",
                            p.number
                        );
                        return StatusCode::OK;
                    }

                    let Some(title) = p.pull_request.title else {
                        error!("Pull request #{} opened without title?!", p.number);
                        return StatusCode::BAD_REQUEST;
                    };
                    state
                        .sender
                        .send(GitHubMessage::PrOpened {
                            pr_id: p.number,
                            pr_title: title,
                            pr_body: p.pull_request.body.and_then(|x| {
                                // multiple long HTML comments in PR template will clutter the embed
                                Some(HTML_COMMENT_REGEX.replace_all(&x, "").to_string())
                            }),
                            opened_by: sender_login,
                        })
                        .await
                        .expect("Failed to send PrOpened message");
                }
                _ => {}
            }
        }

        IssueComment(c) => {
            if c.action != IssueCommentWebhookEventAction::Created
                || c.comment.user.login != c.issue.user.login
            {
                return StatusCode::OK;
            }

            let Some(body) = c.comment.body else {
                error!("Received comment without body: {c:#?}");
                return StatusCode::BAD_REQUEST;
            };

            state
                .sender
                .send(GitHubMessage::AuthorCommented {
                    issue_id: c.issue.number,
                    username: c.comment.user.login,
                    comment: body,
                })
                .await
                .expect("Failed to send PrEdited message");
        }

        _ => {}
    }

    StatusCode::OK
}

async fn server_task(port: u16, webhook_secret: String, sender: mpsc::Sender<GitHubMessage>) {
    let router = Router::new()
        .route("/webhook", post(on_webhook_request))
        .with_state(ServerState {
            sender,
            webhook_secret,
        });

    let listener = match TcpListener::bind(format!("0.0.0.0:{port}")).await {
        Ok(x) => x,
        Err(e) => {
            error!("Webhook server could not bind to port {port}: {e:#?}");
            return;
        }
    };

    info!("Sucessfully bound to port {port}.");
    if let Err(e) = axum::serve(listener, router).await {
        error!("Axum service failed: {e:#?}");
    }
}

impl GitHub {
    /// Initialize the OctoCrab API client and start the webhook server.
    pub async fn initialize(
        webhook_port: u16,
        webhook_secret: String,
        app_config: GhAppConfig,
    ) -> Result<(WebhookServer, GitHub), ()> {
        info!("Initializing octocrab.");
        let octo = match OctocrabBuilder::default()
            .app(app_config.id, app_config.key)
            .build()
        {
            Ok(x) => x,
            Err(e) => {
                error!("Failed to build Octocrab client: {e:#?}");
                return Err(());
            }
        };

        info!(
            "Finding installation for {}/{}",
            app_config.repo_owner, app_config.repo_name
        );

        let install = match octo
            .apps()
            .get_repository_installation(&app_config.repo_owner, &app_config.repo_name)
            .await
        {
            Ok(x) => x,
            Err(e) => {
                error!("Failed to get repository installation: {e:#?}");
                return Err(());
            }
        };

        info!(
            "Got installation with ID {} successfully. Our permissions are {:?}.",
            install.id, install.permissions
        );

        let octo_install = octo
        .installation(install.id)
        .expect("Successfully authorized as GitHub App before. The installation call only returns an error if cached auth data is missing, this shouldn't be happening.");

        let repo = match octo_install
            .repos(&app_config.repo_owner, &app_config.repo_name)
            .get()
            .await
        {
            Ok(x) => x.id,
            Err(e) => {
                error!("Failed to get repository ID: {e}");
                return Err(());
            }
        };
        info!("Got repository ID {repo}.");

        info!("Spawning webhook server task.");
        let (sender, receiver) = mpsc::channel(64);
        let handle = tokio::spawn(server_task(webhook_port, webhook_secret, sender));

        Ok((
            WebhookServer {
                thread: handle,
                receiver,
            },
            GitHub {
                octo_app: octo,
                octo_install: octo_install,
                repo_owner: app_config.repo_owner,
                repo_name: app_config.repo_name,
                repo,
            },
        ))
    }
}
