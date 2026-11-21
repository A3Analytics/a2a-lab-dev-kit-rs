//! Source and capture-time resolution for typed image decisions.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabError, A2aLabService, Image, ImageDecisionExecutor,
    ImageDecisionPrimitiveAnswer, ImageDecisionProviderError, ImageDecisionQuestion,
    ImageDecisionQuestions, ImageDescriptor, ImageId, ImageSource, ImageSourceId, JsonObject,
    MemoryImages, MemoryLogs, MemoryMetrics, MemoryTasks, SourceImageDecisionRequest,
    SourceImageDecisionResult, UtcTimestamp,
};
use async_trait::async_trait;

fn frame(id: &str, captured_at: &str) -> Image {
    let descriptor = ImageDescriptor::new(
        ImageId::new(id).unwrap(),
        ImageSourceId::new("opentrons-camera").unwrap(),
        UtcTimestamp::parse(captured_at).unwrap(),
        "image/jpeg",
        640,
        480,
        None,
        JsonObject::empty(),
    )
    .unwrap();
    Image::new(descriptor, id.as_bytes().to_vec()).unwrap()
}

async fn catalog(frames: Vec<Image>) -> MemoryImages {
    let store = MemoryImages::new();
    store
        .insert_source(ImageSource {
            id: ImageSourceId::new("opentrons-camera").unwrap(),
            name: "Opentrons camera".to_owned(),
            description: "Still frame from the OT-2 on-board camera.".to_owned(),
            asset_id: Some("opentrons-ot2".to_owned()),
            semantic_id: None,
        })
        .await
        .unwrap();
    for frame in frames {
        store.insert_image(frame).await.unwrap();
    }
    store
}

fn questions() -> ImageDecisionQuestions {
    let mut questions = BTreeMap::new();
    questions.insert(
        "tip_attached".to_owned(),
        ImageDecisionQuestion::Noul {
            instructions: "Is a pipette tip attached?".to_owned(),
            criteria: None,
        },
    );
    questions.insert(
        "scene".to_owned(),
        ImageDecisionQuestion::Choice {
            instructions: "What best describes the instrument scene?".to_owned(),
            criteria: BTreeMap::from([
                ("idle".to_owned(), "The instrument is idle.".to_owned()),
                (
                    "liquid_handling".to_owned(),
                    "The instrument is handling liquid.".to_owned(),
                ),
            ]),
        },
    );
    questions.insert(
        "alignment".to_owned(),
        ImageDecisionQuestion::Score {
            instructions: "How well aligned is the pipette tip?".to_owned(),
            criteria: vec!["Poorly aligned".to_owned(), "Well aligned".to_owned()],
        },
    );
    questions
}

fn request(captured_at: &str) -> SourceImageDecisionRequest {
    SourceImageDecisionRequest::new(
        ImageSourceId::new("opentrons-camera").unwrap(),
        UtcTimestamp::parse(captured_at).unwrap(),
        questions(),
    )
    .unwrap()
}

#[derive(Clone)]
struct Recording {
    seen: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl ImageDecisionExecutor for Recording {
    async fn decide(
        &self,
        _request: &a2a_lab_dev_kit::ImageDecisionRequest,
    ) -> Result<a2a_lab_dev_kit::ImageDecisionResult, ImageDecisionProviderError> {
        Err(ImageDecisionProviderError::unavailable("generic path is unused").unwrap())
    }

    async fn decide_questions(
        &self,
        image: &Image,
        questions: &ImageDecisionQuestions,
    ) -> Result<SourceImageDecisionResult, ImageDecisionProviderError> {
        *self.seen.lock().expect("seen") = Some(image.descriptor().id().as_str().to_owned());
        let mut answers = BTreeMap::new();
        answers.insert(
            "tip_attached".to_owned(),
            ImageDecisionPrimitiveAnswer::Noul { noul: 0.94 },
        );
        answers.insert(
            "scene".to_owned(),
            ImageDecisionPrimitiveAnswer::Choice {
                choice: "liquid_handling".to_owned(),
                probabilities: BTreeMap::from([
                    ("idle".to_owned(), 0.06),
                    ("liquid_handling".to_owned(), 0.94),
                ]),
                confidence: 0.91,
            },
        );
        answers.insert(
            "alignment".to_owned(),
            ImageDecisionPrimitiveAnswer::Score {
                score: 1.0,
                legend: vec!["Poorly aligned".to_owned(), "Well aligned".to_owned()],
                probabilities: vec![0.1, 0.9],
                confidence: 0.8,
            },
        );
        SourceImageDecisionResult::from_questions(questions, answers)
    }
}

async fn decide(frames: Vec<Image>, captured_at: &str) -> Result<String, A2aLabError> {
    let seen = Arc::new(Mutex::new(None));
    let lab = A2aLabService::new(MemoryLogs::new(), MemoryMetrics::new(), MemoryTasks::new())
        .with_images(catalog(frames).await)
        .with_image_decision_executor(Arc::new(Recording {
            seen: Arc::clone(&seen),
        }));
    lab.execute(A2aLabCommand::DecideImage(request(captured_at)))
        .await?;
    Ok(seen.lock().expect("seen").clone().expect("selected frame"))
}

#[tokio::test]
async fn exact_capture_beats_a_later_frame_and_ties_take_the_last_id() {
    let selected = decide(
        vec![
            frame("frame-a", "2026-11-20T17:36:00Z"),
            frame("frame-c", "2026-11-20T17:36:01Z"),
            frame("frame-b", "2026-11-20T17:36:00Z"),
        ],
        "2026-11-20T17:36:00Z",
    )
    .await
    .unwrap();
    assert_eq!(selected, "frame-b");
}

#[tokio::test]
async fn missing_exact_frame_uses_the_last_frame_at_the_first_later_time() {
    let selected = decide(
        vec![
            frame("earlier", "2026-11-20T17:35:59Z"),
            frame("after-a", "2026-11-20T17:36:02Z"),
            frame("after-b", "2026-11-20T17:36:02Z"),
            frame("later", "2026-11-20T17:36:03Z"),
        ],
        "2026-11-20T17:36:00Z",
    )
    .await
    .unwrap();
    assert_eq!(selected, "after-b");
}

#[tokio::test]
async fn no_frame_at_or_after_the_capture_time_is_not_found() {
    let error = decide(
        vec![frame("earlier", "2026-11-20T17:35:59Z")],
        "2026-11-20T17:36:00Z",
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "not_found");
}

#[test]
fn public_request_rejects_model_input_image_bytes_and_box_questions() {
    let wire = serde_json::to_value(request("2026-11-20T17:36:00Z")).unwrap();
    assert!(wire.get("image").is_none());
    assert!(wire.get("model").is_none());
    assert!(wire.get("model_input").is_none());

    let mut with_model = wire.clone();
    with_model["model_input"] = serde_json::json!({"model": "jev-1.14.0"});
    assert!(serde_json::from_value::<SourceImageDecisionRequest>(with_model).is_err());

    let mut with_image = wire.clone();
    with_image["image"] = serde_json::json!({"data": "aaaa"});
    assert!(serde_json::from_value::<SourceImageDecisionRequest>(with_image).is_err());

    let mut with_box = wire;
    with_box["questions"]["where"] = serde_json::json!({
        "type": "box",
        "instructions": "Where is the tip?"
    });
    assert!(serde_json::from_value::<SourceImageDecisionRequest>(with_box).is_err());
}
