use fabric::types::frame::FrameRef;

#[test]
fn valid_frame_passes_validation() {
    let f = FrameRef {
        uri: "artifact://sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
            .into(),
        sha256: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".into(),
        mime_type: "image/jpeg".into(),
        width: 640,
        height: 480,
        source_time_ms: 1000,
        camera_id: "cam0".into(),
        frame_id: 1,
    };
    assert!(f.validate().is_ok());
}

#[test]
fn png_is_allowed() {
    let f = FrameRef {
        uri: "artifact://sha256:aaaa".into(),
        sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        mime_type: "image/png".into(),
        width: 1,
        height: 1,
        source_time_ms: 0,
        camera_id: "c".into(),
        frame_id: 0,
    };
    assert!(f.validate().is_ok());
}

#[test]
fn unsupported_mime_rejected() {
    let f = FrameRef {
        mime_type: "image/gif".into(),
        ..valid_frame()
    };
    assert!(f.validate().is_err());
}

#[test]
fn data_uri_rejected() {
    let f = FrameRef {
        uri: "data:image/jpeg;base64,...".into(),
        ..valid_frame()
    };
    assert!(f.validate().is_err());
}

#[test]
fn http_uri_rejected() {
    let f = FrameRef {
        uri: "http://example.com/img.jpg".into(),
        ..valid_frame()
    };
    assert!(f.validate().is_err());
}

#[test]
fn zero_dimensions_rejected() {
    let f = FrameRef {
        width: 0,
        height: 0,
        ..valid_frame()
    };
    assert!(f.validate().is_err());
}

#[test]
fn bad_sha256_rejected() {
    let f = FrameRef {
        sha256: "short".into(),
        ..valid_frame()
    };
    assert!(f.validate().is_err());
}

fn valid_frame() -> FrameRef {
    FrameRef {
        uri: "artifact://sha256:a".into(),
        sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        mime_type: "image/jpeg".into(),
        width: 640,
        height: 480,
        source_time_ms: 1000,
        camera_id: "c".into(),
        frame_id: 0,
    }
}
