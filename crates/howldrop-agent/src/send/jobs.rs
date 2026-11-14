//! Atomic, local send jobs from the Windows UI. Inputs stay in place; only
//! the compressed upload is spooled. Legacy add-on outboxes still take files.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const SUFFIX: &str = ".howldrop-job";
const HEADER: &str = "HowlDrop job 1";

struct Spec {
    to: Option<String>,
    paths: Vec<PathBuf>,
}

fn read(path: &Path) -> io::Result<Spec> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(1_048_577)
        .read_to_string(&mut text)?;
    if text.len() > 1_048_576 {
        return Err(io::Error::other("send job is too large"));
    }
    parse(&text)
}

fn parse(text: &str) -> io::Result<Spec> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(io::Error::other("invalid send job"));
    }
    let to = lines
        .next()
        .ok_or_else(|| io::Error::other("missing recipient"))?;
    let paths: Vec<PathBuf> = lines.map(PathBuf::from).collect();
    if paths.is_empty() || paths.len() > 256 || paths.iter().any(|p| !p.is_absolute()) {
        return Err(io::Error::other(
            "choose 1 to 256 files or folders with absolute paths",
        ));
    }
    Ok(Spec {
        to: (!to.is_empty()).then(|| to.to_owned()),
        paths,
    })
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(super) async fn send_ready(air: &Air, sender: &Sender, opts: &Options, status: &Status) {
    let Ok(entries) = std::fs::read_dir(&opts.outbox) else {
        return;
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.to_string_lossy().ends_with(SUFFIX))
        .collect();
    paths.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    for path in paths {
        let name = file_name(&path);
        let Some(id) = name.strip_suffix(SUFFIX).filter(|id| valid_id(id)) else {
            continue;
        };
        let spec = match read(&path) {
            Ok(spec) => spec,
            Err(e) => {
                emit(&["outgoing", id, "", "1", "0", "Send selection"]);
                finish(opts, &path, id, false, &e.to_string());
                continue;
            }
        };
        let mut fields = vec![
            "outgoing".to_string(),
            id.to_string(),
            spec.to.clone().unwrap_or_default(),
            spec.paths.len().to_string(),
            "0".into(),
        ];
        fields.extend(spec.paths.iter().map(|p| file_name(p)));
        emit(&fields.iter().map(String::as_str).collect::<Vec<_>>());
        let label = if spec.paths.len() == 1 {
            file_name(&spec.paths[0])
        } else {
            format!("{} items", spec.paths.len())
        };
        let job = status.add(&label);
        let progress = |detail: &str| {
            status.update(job, Some("sending"), detail);
            emit(&["outgoing-stage", id, detail]);
        };
        let cancel = opts.outbox.join(format!("{id}.howldrop-cancel"));
        let cancelled = Arc::new(AtomicBool::new(false));
        let operation = async {
            // prepare() also checks every archive block; rejecting links here
            // avoids following a top-level link outside the selected tree.
            for input in &spec.paths {
                let m = std::fs::symlink_metadata(input)?;
                if m.file_type().is_symlink() || !(m.is_file() || m.is_dir()) {
                    return Err(io::Error::other("choose regular files or folders"));
                }
            }
            let _advert = advertise(opts.ble, opts.sender_identity.as_deref()).await;
            let result = try_send_paths(
                air,
                sender,
                &spec.paths,
                spec.to.as_deref().or(opts.to.as_deref()),
                opts.window,
                &progress,
                cancelled.clone(),
                Some(id),
                opts.sender_identity.as_deref(),
            )
            .await;
            Ok::<_, io::Error>(result)
        };
        let stop = async {
            loop {
                if cancel.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        tokio::pin!(operation);
        tokio::pin!(stop);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        let result = loop {
            tokio::select! {
                biased;
                () = &mut stop => {
                    cancelled.store(true, Ordering::Relaxed);
                    break Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
                }
                result = &mut operation => break result,
                _ = heartbeat.tick() => emit(&["outgoing-heartbeat", id]),
            }
        };
        let (ok, detail) = match result {
            Ok((ok, detail, _)) => (ok, detail),
            Err(e) => (false, e.to_string()),
        };
        status.update(job, Some(if ok { "sent" } else { "failed" }), &detail);
        let _ = std::fs::remove_file(&cancel);
        finish(opts, &path, id, ok, &detail);
    }
}

fn finish(opts: &Options, path: &Path, id: &str, ok: bool, detail: &str) {
    // Keep failed manifests for Retry, never copies of the input files.
    let moved = if ok {
        std::fs::remove_file(path)
    } else {
        let failed = opts.outbox.join("failed");
        std::fs::create_dir_all(&failed)
            .and_then(|()| std::fs::rename(path, failed.join(file_name(path))))
    };
    if let Err(e) = moved {
        say(format!("Send: finishing job {id}: {e}"));
    }
    emit(&[
        "outgoing-done",
        id,
        if ok {
            "sent"
        } else if detail == "cancelled" {
            "cancelled"
        } else {
            "failed"
        },
        detail,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jobs_preserve_unicode_spaces_and_grouping() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("holiday photo.jpg");
        let b = dir.path().join("Café");
        let spec = parse(&format!(
            "{HEADER}\nJane’s iPhone\n{}\n{}\n",
            a.display(),
            b.display()
        ))
        .unwrap();
        assert_eq!(spec.to.as_deref(), Some("Jane’s iPhone"));
        assert_eq!(spec.paths, vec![a, b]);
        assert!(parse("HowlDrop job 1\n\nrelative\n").is_err());
        assert!(parse("HowlDrop job 1\n\n").is_err());
        assert!(!valid_id("../escape"));
    }

    #[tokio::test]
    async fn cancelling_a_queued_selection_preserves_its_originals_for_retry() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original.jpg");
        std::fs::write(&original, b"unchanged").unwrap();
        let outbox = dir.path().join("outbox");
        std::fs::create_dir(&outbox).unwrap();
        let id = "0123456789abcdef0123456789abcdef";
        let job = outbox.join(format!("{id}{SUFFIX}"));
        std::fs::write(&job, format!("{HEADER}\n\n{}\n", original.display())).unwrap();
        let cancel = outbox.join(format!("{id}.howldrop-cancel"));
        std::fs::write(&cancel, b"\n").unwrap();
        let opts = Options {
            outbox: outbox.clone(),
            sender_identity: None,
            local_jobs: true,
            discard: true,
            name: "test".into(),
            model: "Mac".into(),
            to: None,
            window: Duration::from_secs(1),
            status: None,
            nearby: None,
            scan_request: None,
            ble: false,
        };
        let (_, peers) = watch::channel(Vec::new());
        let air = Air {
            index: 0,
            peers,
            #[cfg(windows)]
            stack: crate::netstack::Stack::new("fe80::1".parse().unwrap()).0,
        };
        let sender = new_sender(&opts.name, &opts.model, &air, None).unwrap();
        send_ready(&air, &sender, &opts, &Status::new(None)).await;
        assert_eq!(std::fs::read(original).unwrap(), b"unchanged");
        assert!(!job.exists());
        assert!(!cancel.exists());
        assert!(
            outbox
                .join("failed")
                .join(job.file_name().unwrap())
                .exists()
        );
    }
}
