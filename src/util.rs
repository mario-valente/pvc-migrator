use indicatif::{ProgressBar, ProgressStyle};
use std::time::{SystemTime, UNIX_EPOCH};

/// Short, good-enough-unique suffix for pod names (no real uuid crate needed
/// for a prototype that runs one migration at a time).
pub fn short_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", nanos & 0xffffff)
}

/// Byte-based progress bar when the total size is known (always true for
/// restore — it's a local file; for backup, whenever `du` succeeded). Falls
/// back to an indeterminate spinner (shows bytes transferred so far) when
/// the total can't be known upfront.
pub fn progress_bar(total_bytes: Option<u64>, label: &str) -> ProgressBar {
    let pb = match total_bytes {
        Some(total) if total > 0 => {
            let pb = ProgressBar::new(total);
            pb.set_style(
                ProgressStyle::with_template(
                    "{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta})",
                )
                .unwrap()
                .progress_chars("=>-"),
            );
            pb
        }
        _ => {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("{msg} {spinner} {bytes} transferred ({bytes_per_sec})")
                    .unwrap(),
            );
            pb.enable_steady_tick(std::time::Duration::from_millis(120));
            pb
        }
    };
    pb.set_message(label.to_string());
    pb
}
