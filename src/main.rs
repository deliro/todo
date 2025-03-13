use chrono::{Local, Utc};
use clap::{Parser, Subcommand};
use csv::{ReaderBuilder, WriterBuilder};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
#[allow(deprecated)]
use std::env::home_dir;
use std::fmt::{Display, Formatter};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Write, stdin};
use std::path::PathBuf;
use std::{fmt, fs, io};
use strsim::jaro_winkler;
const TODO_STATUS: &str = "todo";
const DONE_STATUS: &str = "done";
const DROPPED_STATUS: &str = "drop";

fn read_line() -> io::Result<String> {
    let mut buf = String::new();
    stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

#[derive(Parser)]
#[command(name = "todo")]
struct TodoCli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print tasks list
    List { status: Option<String> },
    /// Change status to todo
    Todo { task: Vec<String> },
    /// Change status to done
    Done { task: Vec<String> },
    /// Remove a task (soft-delete)
    Drop { task: Vec<String> },
    /// Rename a task
    Rename { task: Vec<String> },
    /// Find tasks
    Find { task: Vec<String> },
    /// Show a task's details (comments)
    Detail { task: Vec<String> },
    /// Add a comment to a task
    Comment { task: Vec<String> },
    /// Create new task
    #[clap(external_subcommand)]
    External(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Task {
    id: usize,
    status: String,
    title: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    comments: String,
}

impl Display for Task {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}. {}", self.id, self.title)
    }
}

impl Task {
    fn details(&self) -> Result<String, fmt::Error> {
        use std::fmt::Write;

        let mut buf = String::with_capacity(128);
        writeln!(buf, "Title: {}", self.title)?;
        writeln!(buf, "ID: {}", self.id)?;
        writeln!(buf, "Status: {}", self.status)?;
        if !self.comments.is_empty() {
            writeln!(buf, "----- comments -----")?;
            for comment in self.comments.lines() {
                writeln!(buf, "{comment}")?;
            }
            writeln!(buf, "--------------------")?;
        }
        writeln!(
            buf,
            "created at: {:?}",
            self.created_at.with_timezone(&Local)
        )?;
        write!(
            buf,
            "updated at: {:?}",
            self.updated_at.with_timezone(&Local)
        )?;
        Ok(buf)
    }

    fn change_title(&mut self, new_title: String) {
        self.title = new_title;
        self.updated_at = Utc::now();
    }

    fn add_comment(&mut self, comment: &str) {
        if !self.comments.is_empty() {
            self.comments.push_str("\n\n");
        }
        self.comments.push_str(comment);
        self.updated_at = Utc::now();
    }

    fn set_status(&mut self, status: String) -> Option<&Task> {
        self.status = status;
        self.updated_at = Utc::now();
        Some(self)
    }
}

struct Tasks {
    inner: Vec<Task>,
    filename: PathBuf,
}

fn is_candidate(needle: &str, needle_words: &BTreeSet<&str>, task: &Task) -> bool {
    debug_assert!(needle.to_lowercase() == needle);
    debug_assert!(needle_words.iter().all(|w| w.to_lowercase() == *w));

    let title = task.title.to_lowercase();
    if needle == title {
        return true;
    }
    if title.contains(needle) {
        return true;
    }
    if needle_words.iter().all(|w| title.contains(w)) {
        return true;
    }

    let title_words = title.split_whitespace().collect::<BTreeSet<&str>>();
    let mut weights = vec![];
    for needle_word in needle_words {
        for title_word in &title_words {
            weights.push((
                jaro_winkler(needle_word, title_word),
                needle_word,
                title_word,
            ))
        }
    }
    weights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Less));
    weights.reverse();
    if weights
        .iter()
        .any(|(x, needle, title)| x >= &0.999 && (needle.len() >= 3 || title.len() >= 3))
    {
        return true;
    }
    let sum: f64 = weights
        .iter()
        .take(needle_words.len())
        .map(|(x, _, _)| x)
        .sum();
    let count = (needle_words.len().saturating_sub(1) + 1) as f64;
    if (sum / count) > 0.85 {
        return true;
    }
    false
}

impl Tasks {
    fn load_default() -> io::Result<Self> {
        #[allow(deprecated)]
        let mut file = home_dir().expect("cannot determine home directory");
        file.push(".todo");
        file.push("tasks.csv");
        Self::load(file)
    }

    fn load(filename: PathBuf) -> io::Result<Self> {
        if let Some(dir) = filename.parent() {
            fs::create_dir_all(dir)?
        }
        let file = File::open(&filename).or_else(|_| {
            OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .read(true)
                .open(&filename)
        })?;
        let reader = BufReader::new(file);
        let mut rdr = ReaderBuilder::new().has_headers(true).from_reader(reader);
        let mut tasks = vec![];
        for r in rdr.deserialize() {
            tasks.push(r?)
        }
        Ok(Self {
            inner: tasks,
            filename,
        })
    }

    fn find_idx(&self, id: usize) -> Option<usize> {
        self.inner
            .iter()
            .enumerate()
            .find_map(|(idx, t)| (t.id == id).then_some(idx))
    }

    fn set_status(&mut self, id: usize, status: String) -> Option<&Task> {
        let idx = self.find_idx(id)?;
        let task = self.inner.get_mut(idx)?;
        task.set_status(status)
    }

    fn set_done(&mut self, id: usize) -> Option<&Task> {
        self.set_status(id, DONE_STATUS.into())
    }

    fn set_todo(&mut self, id: usize) -> Option<&Task> {
        self.set_status(id, TODO_STATUS.into())
    }

    fn set_dropped(&mut self, id: usize) -> Option<&Task> {
        self.set_status(id, DROPPED_STATUS.into())
    }

    fn next_id(&self) -> usize {
        self.inner.iter().map(|t| t.id).max().unwrap_or(0) + 1
    }

    fn todo(&mut self, title: String, comments: String) -> &Task {
        let id = self.next_id();
        let task = Task {
            id,
            title,
            comments,
            status: TODO_STATUS.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.inner.push(task);
        self.inner.iter().last().unwrap()
    }

    fn save(&self) -> io::Result<()> {
        let buf = {
            let mut wtr = WriterBuilder::new().has_headers(true).from_writer(vec![]);
            for record in &self.inner {
                wtr.serialize(record)?;
            }
            wtr.into_inner()
                .map_err(|_| io::Error::other("cannot flush the buffer"))?
        };
        if let Some(dir) = self.filename.parent() {
            fs::create_dir_all(dir)?
        }
        let mut file = File::create(&self.filename)?;
        file.write_all(&buf)?;
        Ok(())
    }

    fn find_id(&self, id: usize) -> Option<&Task> {
        self.inner.iter().find(|t| t.id == id)
    }

    fn find_id_mut(&mut self, id: usize) -> Option<&mut Task> {
        self.inner.iter_mut().find(|t| t.id == id)
    }

    fn find(&self, name_or_id: &str, show_dropped: bool) -> Vec<&Task> {
        let name_or_id = name_or_id.trim();
        if name_or_id.is_empty() {
            return vec![];
        }
        let mut candidates = vec![];
        if let Ok(id) = name_or_id.parse::<usize>() {
            if let Some(task) = self.iter().find(|t| t.id == id) {
                candidates.push(task)
            }
        }

        let needle = name_or_id.to_lowercase();
        let needle_words = needle.split_whitespace().collect::<BTreeSet<&str>>();

        for task in self.iter() {
            if is_candidate(&needle, &needle_words, task) {
                candidates.push(task)
            }
        }

        if !show_dropped {
            candidates.retain(|t| t.status != DROPPED_STATUS)
        }
        candidates.dedup_by(|t, t2| t.id == t2.id);
        candidates
    }

    fn select_interactive(&self, needle: &str, show_dropped: bool) -> Option<usize> {
        let ids: Vec<_> = self
            .find(needle, show_dropped)
            .iter()
            .map(|t| t.id)
            .collect();
        match ids.as_slice() {
            [] => None,
            [id] => Some(*id),
            many => {
                println!("Select ID:");
                let matched = self.iter().filter(|t| many.contains(&t.id));
                print_tasks(matched, None);
                let id: usize = read_line().ok()?.parse().ok()?;
                self.find_id(id).is_some().then_some(id)
            }
        }
    }

    fn iter(&self) -> impl Iterator<Item = &Task> {
        self.inner.iter()
    }
}

fn print_tasks<'a>(tasks: impl Iterator<Item = &'a Task> + 'a, only_status: Option<String>) {
    let mut by_status: HashMap<String, Vec<&Task>> = HashMap::new();
    for task in tasks {
        by_status.entry(task.status.clone()).or_default().push(task)
    }

    let statuses = match only_status {
        Some(v) => vec![v],
        None => vec![TODO_STATUS.into(), DONE_STATUS.into()],
    };

    for status in statuses {
        if let Some(status_tasks) = by_status.get(&status) {
            println!("[{status}]:");
            for task in status_tasks {
                println!("{}", task)
            }
        }
    }
}

macro_rules! print_not_found {
    () => {
        println!("Not found")
    };
}

fn main() -> io::Result<()> {
    let cli = TodoCli::parse();
    match cli.command {
        Command::List { status } => {
            let tasks = Tasks::load_default()?;
            print_tasks(tasks.iter(), status);
        }
        Command::Done { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task, false)
                .and_then(|id| tasks.set_done(id))
            {
                None => print_not_found!(),
                Some(t) => println!("Done: {}", t),
            }
            tasks.save()?;
        }
        Command::Todo { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task, false)
                .and_then(|id| tasks.set_todo(id))
            {
                None => print_not_found!(),
                Some(t) => println!("TODO: {}", t),
            }
            tasks.save()?;
        }
        Command::Drop { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task, false)
                .and_then(|id| tasks.set_dropped(id))
            {
                None => print_not_found!(),
                Some(t) => println!("Dropped: {}", t),
            }
            tasks.save()?;
        }
        Command::External(args) => {
            let mut tasks = Tasks::load_default()?;
            let title = args.join(" ");
            let id = tasks.todo(title, "".into()).id;
            tasks.save()?;
            let task = tasks.find_id(id).unwrap();
            println!("Task {} has been created", task);
        }
        Command::Find { task } => {
            let task = task.join(" ");
            let tasks = Tasks::load_default()?;
            let matched = tasks.find(&task, false);
            print_tasks(matched.into_iter(), None)
        }
        Command::Detail { task } => {
            let task = task.join(" ");
            let tasks = Tasks::load_default()?;

            match tasks
                .select_interactive(&task, true)
                .and_then(|id| tasks.find_id(id))
            {
                None => print_not_found!(),
                Some(task) => {
                    let details = task.details().unwrap();
                    println!("{details}");
                }
            }
        }
        Command::Comment { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;

            match tasks
                .select_interactive(&task, false)
                .and_then(|id| tasks.find_id_mut(id))
            {
                None => print_not_found!(),
                Some(task) => {
                    println!("Comment for {}:", task);
                    let comment = read_line()?;
                    if !comment.is_empty() {
                        task.add_comment(&comment)
                    }
                }
            }

            tasks.save()?
        }
        Command::Rename { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task, false)
                .and_then(|id| tasks.find_id_mut(id))
            {
                None => print_not_found!(),
                Some(task) => {
                    println!("New name:");
                    let new_title = read_line()?;
                    task.change_title(new_title)
                }
            }
            tasks.save()?
        }
    }
    Ok(())
}
