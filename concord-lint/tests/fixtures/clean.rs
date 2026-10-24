use tracing::{info, warn};

fn example(id: &str, room: &str) {
    info!(action_id = %id, "sent");
    warn!(room_id = %room, "late");
}
async fn multi_line_and_generics(id: &str) {
    let task = async {
        info!(
            action_id = %id,
            context = foo::<A, B>(),
            "sent"
        );
    };
    task.await;
}
