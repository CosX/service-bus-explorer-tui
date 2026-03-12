use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::app::BgEvent;
use crate::client::{DataPlaneClient, ManagementClient};

pub fn send_path_owned(entity_path: &str) -> String {
    crate::client::entity_path::send_target(entity_path).to_string()
}

pub async fn resolve_purge_paths(
    mgmt: Option<&ManagementClient>,
    entity_path: &str,
    is_topic: bool,
    is_dlq: bool,
) -> Result<Vec<String>, String> {
    if is_topic {
        let mgmt = mgmt.ok_or_else(|| "Not connected".to_string())?;
        let subs = mgmt
            .list_subscriptions(entity_path)
            .await
            .map_err(|e| format!("Failed to list subscriptions: {}", e))?;
        Ok(subs
            .iter()
            .map(|s| {
                let sub_path = format!("{}/subscriptions/{}", entity_path, s.name);
                if is_dlq {
                    format!("{}/$deadletterqueue", sub_path)
                } else {
                    sub_path
                }
            })
            .collect())
    } else if is_dlq {
        Ok(vec![format!("{}/$deadletterqueue", entity_path)])
    } else {
        Ok(vec![entity_path.to_string()])
    }
}

pub async fn resolve_resend_pairs(
    mgmt: Option<&ManagementClient>,
    entity_path: &str,
    send_target: &str,
    is_topic: bool,
) -> Result<Vec<(String, String)>, String> {
    if is_topic {
        let mgmt = mgmt.ok_or_else(|| "Not connected".to_string())?;
        let subs = mgmt
            .list_subscriptions(entity_path)
            .await
            .map_err(|e| format!("Failed to list subscriptions: {}", e))?;
        Ok(subs
            .iter()
            .map(|s| {
                let dlq = format!("{}/subscriptions/{}/$deadletterqueue", entity_path, s.name);
                (dlq, send_target.to_string())
            })
            .collect())
    } else {
        let dlq = format!("{}/$deadletterqueue", entity_path);
        Ok(vec![(dlq, send_target.to_string())])
    }
}

pub async fn resend_dlq_loop(
    dp: &DataPlaneClient,
    pairs: &[(String, String)],
    max_per_path: Option<u32>,
    cancel: &Arc<AtomicBool>,
    tx: &UnboundedSender<BgEvent>,
) -> Result<(u32, u32), String> {
    let mut resent = 0u32;
    let mut errors = 0u32;

    for (dlq_path, send_target) in pairs {
        let mut path_count = 0u32;
        loop {
            if let Some(max) = max_per_path {
                if path_count >= max {
                    break;
                }
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(format!(
                    "Cancelled after resending {} messages ({} errors)",
                    resent, errors
                ));
            }

            let locked = match dp.peek_lock(dlq_path, 1).await {
                Ok(Some(msg)) => msg,
                Ok(None) => break,
                Err(e) => return Err(format!("Resend failed after {} messages: {}", resent, e)),
            };

            let lock_uri = match locked.lock_token_uri {
                Some(ref uri) => uri.clone(),
                None => {
                    errors += 1;
                    path_count += 1;
                    continue;
                }
            };

            match dp.send_message(send_target, &locked.to_sendable()).await {
                Ok(_) => {
                    if dp.complete_message(&lock_uri).await.is_ok() {
                        resent += 1;
                    } else {
                        errors += 1;
                    }
                }
                Err(_) => {
                    let _ = dp.abandon_message(&lock_uri).await;
                    errors += 1;
                }
            }

            path_count += 1;
            if (resent + errors).is_multiple_of(50) {
                let _ = tx.send(BgEvent::Progress(format!(
                    "Resent {} messages ({} errors)... (Esc to cancel)",
                    resent, errors
                )));
            }
        }
    }

    Ok((resent, errors))
}
