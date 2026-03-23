use crate::SendWebhook;
use sqlx::PgPool;


pub struct Db {
    pool: PgPool,
}


    // "I never write more than 10 lines without testing it"
pub async fn write_to_dlq(job: &SendWebhook) {

    println!("Writing to dlq...");
}

//     pub async fn write_to_dlq(self, job: &SendWebhook, tenant_id: String) {
//         let payload = serde_json::to_string(&job.payload).unwrap();

//         sqlx::query!(
//             r#"
//             INSERT INTO dead_letter_queue
//                 (id, tenant_id, destination, payload, abandoned_at, failure_reason)
//             VALUES ($1, $2, $3, $4, $5, $6) 
//             "#,
//             job.id,
//             tenant_id,

//         )
//         .execute(&self.pool)
//         .await?;

//         Ok(())
//     }

// }