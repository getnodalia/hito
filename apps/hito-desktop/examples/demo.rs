//! Fills a local Hito server with a demo company, for screenshots and for trying the app:
//! three people (Ana, Luis and Marta), a workspace, three projects with milestones, tasks,
//! checklists and comments, all sealed end to end like real ones.
//!
//! ```sh
//! cargo run -p hito-server &                     # http://127.0.0.1:7879, open registration
//! cargo run -p hito-server -- init-token          # prints the setup token for the first account
//! HITO_INIT_TOKEN=<token> HITO_DESKTOP_DIR=/tmp/hito-demo cargo run -p hito-desktop --example demo
//! HITO_DESKTOP_DIR=/tmp/hito-demo cargo run -p hito-desktop   # opens signed in as Ana
//! ```
//!
//! Every account's password is `demo-password-1`. Run it against a fresh database: the
//! usernames must be free.

use std::error::Error;
use std::path::PathBuf;

use hito_client::{Client, Secure};
use hito_core::{Priority, ProjectId, Role, Status, UserId};
use hito_protocol::{
    ChecklistItem, CreateInviteRequest, CreateMilestoneRequest, CreateTaskRequest,
    MilestoneContent, MoveTaskRequest, RegisterRequest, TaskContent, UpdateProjectRequest,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const PASSWORD: &str = "demo-password-1";
const DAY: i64 = 24 * 60 * 60 * 1000;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

struct Person {
    client: Client,
    secure: Secure,
    id: UserId,
}

fn dir() -> Option<PathBuf> {
    std::env::var_os("HITO_DESKTOP_DIR").map(PathBuf::from)
}

fn device(server: &str, user: UserId, who: &str) -> Option<PathBuf> {
    let host: String = server
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    // Ana's device goes where the desktop app looks for it; the others are kept beside it.
    let name = if who == "ana" {
        format!("device-{host}-{}.json", user.0)
    } else {
        format!("demo-{who}.json")
    };
    Some(dir()?.join(name))
}

async fn register(
    server: &Client,
    username: &str,
    name: &str,
    code: Option<String>,
) -> Result<Person> {
    let (client, session) = server
        .register(&RegisterRequest {
            username: username.to_string(),
            display_name: name.to_string(),
            password: PASSWORD.to_string(),
            invite_code: code,
        })
        .await?;
    let id = session.user.id;
    let path = device(client.base().as_str(), id, username);
    let secure = Secure::open(&client, id, path).await?;
    Ok(Person { client, secure, id })
}

struct NewTask<'a> {
    title: &'a str,
    status: Status,
    priority: Priority,
    assignee: Option<UserId>,
    milestone: Option<usize>,
    due_in_days: Option<i64>,
    labels: &'a [&'a str],
    checklist: &'a [(&'a str, bool)],
    description: &'a str,
}

impl Default for NewTask<'_> {
    fn default() -> Self {
        NewTask {
            title: "",
            status: Status::Todo,
            priority: Priority::Normal,
            assignee: None,
            milestone: None,
            due_in_days: None,
            labels: &[],
            checklist: &[],
            description: "",
        }
    }
}

async fn project(
    owner: &Person,
    ws: hito_core::WorkspaceId,
    name: &str,
    description: &str,
    milestones: &[(&str, i64)],
    tasks: &[NewTask<'_>],
) -> Result<(ProjectId, Vec<hito_core::TaskId>)> {
    let client = &owner.client;
    let project = client.create_project(ws, name).await?;
    owner.secure.sync(project.id).await?;
    let sealed = owner
        .secure
        .seal_description(project.id, description)
        .await?;
    client
        .update_project(
            project.id,
            &UpdateProjectRequest {
                description: Some(sealed),
                ..Default::default()
            },
        )
        .await?;
    let mut ids = Vec::new();
    for (m, days) in milestones {
        let content = MilestoneContent {
            name: m.to_string(),
            description: String::new(),
        };
        let sealed = owner.secure.seal_milestone(project.id, &content).await?;
        let created = client
            .create_milestone(
                project.id,
                &CreateMilestoneRequest {
                    content: sealed,
                    due_at: Some(now() + days * DAY),
                },
            )
            .await?;
        ids.push(created.id);
    }
    let mut task_ids = Vec::new();
    for t in tasks {
        let content = TaskContent {
            title: t.title.to_string(),
            description: t.description.to_string(),
            labels: t.labels.iter().map(|l| l.to_string()).collect(),
            checklist: t
                .checklist
                .iter()
                .map(|(text, done)| ChecklistItem {
                    text: text.to_string(),
                    done: *done,
                })
                .collect(),
        };
        let sealed = owner.secure.seal_task(project.id, &content).await?;
        let task = client
            .create_task(
                project.id,
                &CreateTaskRequest {
                    content: sealed,
                    status: Some(Status::Todo),
                    priority: Some(t.priority),
                    assignee_id: t.assignee,
                    milestone_id: t.milestone.and_then(|i| ids.get(i).copied()),
                    due_at: t.due_in_days.map(|d| now() + d * DAY),
                },
            )
            .await?;
        if t.status != Status::Todo {
            client
                .move_task(
                    task.id,
                    &MoveTaskRequest {
                        status: t.status,
                        after: None,
                        before: None,
                    },
                )
                .await?;
        }
        task_ids.push(task.id);
    }
    Ok((project.id, task_ids))
}

#[tokio::main]
async fn main() -> Result<()> {
    let base = std::env::var("HITO_SERVER").unwrap_or_else(|_| "http://127.0.0.1:7879".into());
    let server = Client::new(base.parse()?)?;

    let setup = std::env::var("HITO_INIT_TOKEN").ok();
    let ana = register(&server, "ana", "Ana García", setup).await?;
    let ws = ana.client.create_workspace("Nodalia Studio").await?.id;
    let invite = ana
        .client
        .create_invite(
            ws,
            &CreateInviteRequest {
                max_uses: Some(2),
                expires_in_hours: Some(24),
                role: Some(Role::Member),
            },
        )
        .await?;
    let luis = register(&server, "luis", "Luis Romero", Some(invite.code.clone())).await?;
    let marta = register(&server, "marta", "Marta Iglesias", Some(invite.code)).await?;

    use Priority::*;
    use Status::*;
    let (launch, launch_tasks) = project(
        &ana,
        ws,
        "Hito 1.0 launch",
        "Everything between the beta and the public release.",
        &[("Beta ready", -3), ("Release candidate", 9), ("Public launch", 23)],
        &[
            NewTask { title: "Freeze the feature list", status: Done, milestone: Some(0), assignee: Some(ana.id), ..Default::default() },
            NewTask { title: "Beta with ten companies", status: Done, milestone: Some(0), assignee: Some(luis.id), ..Default::default() },
            NewTask {
                title: "Write the release notes",
                status: InProgress,
                priority: High,
                milestone: Some(1),
                assignee: Some(marta.id),
                due_in_days: Some(4),
                labels: &["docs"],
                checklist: &[("New features", true), ("Fixes", true), ("Screenshots", false), ("Translations", false)],
                description: "One page, in plain words. Lead with what people can do now that they could not before.",
            },
            NewTask { title: "Fix the reminders on Windows", status: InReview, priority: Urgent, milestone: Some(1), assignee: Some(luis.id), due_in_days: Some(1), labels: &["bug", "windows"], ..Default::default() },
            NewTask { title: "Record the demo video", status: InProgress, milestone: Some(2), assignee: Some(ana.id), due_in_days: Some(12), labels: &["marketing"], ..Default::default() },
            NewTask { title: "Translate the website", milestone: Some(2), assignee: Some(marta.id), due_in_days: Some(18), labels: &["web"], ..Default::default() },
            NewTask { title: "Press kit", milestone: Some(2), priority: Low, due_in_days: Some(20), labels: &["marketing"], ..Default::default() },
            NewTask { title: "Load test the server", status: Todo, priority: High, milestone: Some(1), assignee: Some(luis.id), due_in_days: Some(-1), labels: &["server"], ..Default::default() },
        ],
    )
    .await?;
    project(
        &ana,
        ws,
        "Website redesign",
        "A calmer site, faster pages and the new pricing.",
        &[("Design approved", 6), ("New site live", 30)],
        &[
            NewTask {
                title: "Moodboard and type",
                status: Done,
                milestone: Some(0),
                assignee: Some(marta.id),
                ..Default::default()
            },
            NewTask {
                title: "Home page layout",
                status: InReview,
                milestone: Some(0),
                assignee: Some(marta.id),
                due_in_days: Some(2),
                labels: &["design"],
                ..Default::default()
            },
            NewTask {
                title: "Pricing page copy",
                status: InProgress,
                milestone: Some(0),
                assignee: Some(ana.id),
                due_in_days: Some(5),
                ..Default::default()
            },
            NewTask {
                title: "Move the blog",
                milestone: Some(1),
                priority: Low,
                ..Default::default()
            },
        ],
    )
    .await?;
    project(
        &ana,
        ws,
        "Design fair stand",
        "Our stand at the spring design fair.",
        &[("Stand booked", 10)],
        &[
            NewTask {
                title: "Book the stand",
                status: InProgress,
                priority: High,
                milestone: Some(0),
                assignee: Some(ana.id),
                due_in_days: Some(3),
                ..Default::default()
            },
            NewTask {
                title: "Print the banners",
                milestone: Some(0),
                due_in_days: Some(9),
                ..Default::default()
            },
        ],
    )
    .await?;

    // The others' devices receive the keys, then they comment like real teammates.
    for p in [&luis, &marta] {
        p.secure.sync(launch).await?;
    }
    if let Some(notes) = launch_tasks.get(2) {
        let body = marta
            .secure
            .seal_comment(
                launch,
                "Draft is in the shared folder. Screenshots tomorrow.",
            )
            .await?;
        marta.client.post_comment(*notes, body).await?;
        let body = luis
            .secure
            .seal_comment(
                launch,
                "Mention the new webhooks for Ágora, people asked for it.",
            )
            .await?;
        luis.client.post_comment(*notes, body).await?;
    }

    if let (Some(dir), Some(token)) = (dir(), ana.client.token()) {
        std::fs::create_dir_all(&dir)?;
        let session = serde_json::json!({ "server": ana.client.base().as_str(), "token": token });
        std::fs::write(dir.join("session.json"), session.to_string())?;
    }
    println!("Demo ready: sign in as ana, luis or marta with {PASSWORD}.");
    Ok(())
}
