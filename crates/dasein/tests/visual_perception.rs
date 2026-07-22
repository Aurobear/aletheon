use dasein::r#impl::perception::visual_aggregator::{VisualAggregator, VisualAggregatorConfig};

#[test]
fn integration_dedup_and_rate_limit() {
    let mut agg = VisualAggregator::new(VisualAggregatorConfig {
        max_hz: 2,
        max_age_ms: 10000,
        max_frames_per_camera: 4,
        max_total_events: 16,
    });
    let mut count = 0;
    for i in 0..10 {
        use fabric::types::frame::FrameRef;
        let f = FrameRef {
            uri: format!("artifact://sha256:{}", i),
            sha256: format!("{:064x}", i),
            mime_type: "image/jpeg".into(),
            width: 640,
            height: 480,
            source_time_ms: 1000,
            camera_id: "cam0".into(),
            frame_id: i,
        };
        if agg.ingest(
            f,
            vec![],
            format!("frame_{}", i),
            0.9,
            1000,
        )
        .is_some()
        {
            count += 1;
        }
    }
    // At 2 Hz, should only accept 2 frames
    assert_eq!(count, 2);
}
