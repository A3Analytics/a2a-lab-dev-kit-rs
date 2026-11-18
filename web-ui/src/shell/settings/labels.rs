//! Display names for settings field paths.

pub(super) fn friendly_label(path: &str) -> String {
    let name = path.rsplit('.').next().unwrap_or(path);
    known_label(name)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| title_case(name))
}

fn known_label(name: &str) -> Option<&'static str> {
    match name {
        "credentials" => Some("Credentials"),
        "env" => Some("Environment variable"),
        "fps" => Some("FPS"),
        "dvr" => Some("DVR"),
        "roi" => Some("ROI"),
        "top_k" => Some("Top K"),
        "max_batch_items" => Some("Maximum batch items"),
        "max_concurrency" => Some("Maximum concurrency"),
        "max_queue_backlog" => Some("Maximum queue backlog"),
        "max_retry_attempts" => Some("Maximum retry attempts"),
        "max_neighbor_frames" => Some("Maximum neighbor frames"),
        "max_rois_per_frame" => Some("Maximum ROIs per frame"),
        "job_deadline" => Some("Job deadline"),
        "retry_backoff" => Some("Retry backoff"),
        "score_floor" => Some("Score floor"),
        "sqlite_path" => Some("SQLite path"),
        "vector_index_path" => Some("Vector index path"),
        "max_disk_bytes" => Some("Maximum disk size"),
        "capture_rate" => Some("Capture rate"),
        "storage_path" => Some("Storage path"),
        _ => None,
    }
}

fn title_case(name: &str) -> String {
    name.split('_')
        .map(capitalize_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_label_maps_acronyms_and_special_names() {
        assert_eq!(friendly_label("videos.fps"), "FPS");
        assert_eq!(friendly_label("env"), "Environment variable");
    }

    #[test]
    fn friendly_label_maps_maximum_fields() {
        assert_eq!(
            friendly_label("storage.max_disk_bytes"),
            "Maximum disk size"
        );
    }

    #[test]
    fn friendly_label_title_cases_unknown_snake_case() {
        assert_eq!(friendly_label("camera.sample_rate"), "Sample Rate");
    }
}
