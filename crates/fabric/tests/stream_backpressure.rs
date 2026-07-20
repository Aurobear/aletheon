use fabric::ipc::stream::{BoundedStream, OverflowPolicy, StreamSendError, StreamSpec};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn drop_oldest_keeps_newest_items() {
    let mut stream = BoundedStream::new(StreamSpec::telemetry(2));
    stream.send(1).await.unwrap();
    stream.send(2).await.unwrap();
    stream.send(3).await.unwrap();

    assert_eq!(stream.try_recv(), Some(2));
    assert_eq!(stream.try_recv(), Some(3));
    assert_eq!(stream.try_recv(), None);
}

#[tokio::test]
async fn drop_newest_preserves_existing_items() {
    let mut stream = BoundedStream::new(StreamSpec {
        capacity: 1,
        overflow: OverflowPolicy::DropNewest,
        cancel: CancellationToken::new(),
    });
    stream.send("old").await.unwrap();
    stream.send("new").await.unwrap();
    assert_eq!(stream.try_recv(), Some("old"));
    assert_eq!(stream.try_recv(), None);
}

#[tokio::test]
async fn fail_stream_reports_overflow() {
    let mut stream = BoundedStream::new(StreamSpec {
        capacity: 1,
        overflow: OverflowPolicy::FailStream,
        cancel: CancellationToken::new(),
    });
    stream.send(1).await.unwrap();
    assert_eq!(stream.send(2).await, Err(StreamSendError::Overflow));
}

#[tokio::test]
async fn cancelled_stream_rejects_send() {
    let cancel = CancellationToken::new();
    let mut stream = BoundedStream::<u8>::new(StreamSpec {
        capacity: 1,
        overflow: OverflowPolicy::BlockProducer,
        cancel: cancel.clone(),
    });
    cancel.cancel();
    assert_eq!(stream.send(1).await, Err(StreamSendError::Cancelled));
}
