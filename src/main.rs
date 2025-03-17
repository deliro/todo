use atty::Stream;
use chrono::{Local, Utc};
use clap::{Parser, Subcommand};
use csv::{ReaderBuilder, WriterBuilder};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
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

fn read_lines() -> io::Result<String> {
    if atty::is(Stream::Stdin) {
        let mut buf = String::new();
        stdin().read_line(&mut buf)?;
        Ok(buf.trim().to_string())
    } else {
        let mut buf = String::new();
        for line in stdin().lines() {
            buf.push_str(&line?);
            buf.push_str("\n");
        }
        Ok(buf.trim().to_string())
    }
}

trait StringExt {
    fn contains_all<T: AsRef<str>>(&self, i: impl IntoIterator<Item = T>) -> bool;
}

impl<T> StringExt for T
where
    T: AsRef<str>,
{
    fn contains_all<Item: AsRef<str>>(&self, i: impl IntoIterator<Item = Item>) -> bool {
        i.into_iter().all(|x| self.as_ref().contains(x.as_ref()))
    }
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
    #[clap(visible_aliases = &["l", "ls"])]
    List { status: Option<String> },
    /// Change status to todo
    Todo { task: Vec<String> },
    /// Change status to done
    Done { task: Vec<String> },
    /// Remove a task (soft-delete)
    #[clap(visible_aliases = &["remove", "delete", "rm"])]
    Drop { task: Vec<String> },
    /// Rename a task
    Rename { task: Vec<String> },
    /// Find tasks
    Find { task: Vec<String> },
    /// Show a task's details (comments)
    #[clap(visible_alias = "d")]
    Detail { task: Vec<String> },
    /// Add a comment to a task
    Comment { task: Vec<String> },
    /// Physically remove all dropped tasks
    RemoveDropped,
    /// Prints the tasks file path
    Where,
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

    fn set_status(&mut self, status: String) {
        self.status = status;
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
struct Idx(usize);

impl From<usize> for Idx {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<Idx> for usize {
    fn from(value: Idx) -> Self {
        value.0
    }
}

#[derive(Debug, Copy, Clone)]
struct Loc {
    idx: Idx,
    id: usize,
}

impl Loc {
    fn new<I: Into<Idx>>(idx: I, id: usize) -> Self {
        Self {
            idx: idx.into(),
            id,
        }
    }
}

struct Tasks {
    inner: Vec<Task>,
    filename: PathBuf,
}

impl Tasks {
    fn default_path() -> PathBuf {
        #[allow(deprecated)]
        let mut file = home_dir().expect("cannot determine home directory");
        file.push(".todo");
        file.push("tasks.csv");
        file
    }
    fn load_default() -> io::Result<Self> {
        Self::load(Self::default_path())
    }

    fn load(filename: PathBuf) -> io::Result<Self> {
        log::debug!("loading tasks from {filename:?}");
        if let Some(dir) = filename.parent() {
            fs::create_dir_all(dir)?;
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
            tasks.push(r?);
        }
        Ok(Self {
            inner: tasks,
            filename,
        })
    }

    fn set_status_idx(&mut self, idx: Idx, status: String) -> Option<&Task> {
        let task = self.find_idx_mut(idx)?;
        task.set_status(status);
        Some(task)
    }

    fn set_done_idx(&mut self, idx: Idx) -> Option<&Task> {
        self.set_status_idx(idx, DONE_STATUS.into())
    }

    fn set_todo_idx(&mut self, idx: Idx) -> Option<&Task> {
        self.set_status_idx(idx, TODO_STATUS.into())
    }

    fn set_dropped_idx(&mut self, idx: Idx) -> Option<&Task> {
        self.set_status_idx(idx, DROPPED_STATUS.into())
    }

    fn remove_dropped(&mut self) -> usize {
        let orig_len = self.inner.len();
        self.inner.retain(|t| t.status != DROPPED_STATUS);
        let new_len = self.inner.len();
        orig_len - new_len
    }

    fn next_loc(&self) -> Loc {
        // We might have assumed the last vec element is the latest hence has the
        // greatest ID, but the tasks file may be externally shuffled so seq scan
        // is the only option.
        let next_id = self.inner.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        let next_idx = self.inner.len();
        Loc::new(next_idx, next_id)
    }

    fn todo(&mut self, title: String, comments: String) -> Loc {
        let loc = self.next_loc();
        debug_assert_eq!(loc.idx, self.inner.len().into());
        let task = Task {
            id: loc.id,
            title,
            comments,
            status: TODO_STATUS.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.inner.push(task);
        loc
    }

    fn save(&self) -> io::Result<()> {
        let buf = {
            log::debug!("writing tasks to buffer before saving to file");
            let mut wtr = WriterBuilder::new().has_headers(true).from_writer(vec![]);
            for record in &self.inner {
                wtr.serialize(record)?;
            }
            wtr.into_inner()
                .map_err(|_| io::Error::other("cannot flush the buffer"))?
        };
        if let Some(dir) = self.filename.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = File::create(&self.filename)?;
        file.write_all(&buf)?;
        log::debug!("file saved");
        Ok(())
    }

    fn find_idx(&self, idx: Idx) -> Option<&Task> {
        let i: usize = idx.into();
        self.inner.get(i)
    }

    fn find_idx_mut(&mut self, idx: Idx) -> Option<&mut Task> {
        let i: usize = idx.into();
        self.inner.get_mut(i)
    }

    fn find(&self, needle: &str, show_dropped: bool) -> Vec<(Loc, &Task)> {
        let mut candidates = vec![];
        log::debug!("searching candidates for '{needle}'");
        for (idx, task) in self.iter().enumerate() {
            let candidate = Candidate::check(needle, task);
            log::debug!("candidate '{task}' result is {candidate:?}");
            match candidate {
                Candidate::ById if show_dropped || task.status != DROPPED_STATUS => {
                    log::debug!("searching stopped because ID was found");
                    return vec![(Loc::new(idx, task.id), task)];
                }
                Candidate::No => {}
                _ => candidates.push((Loc::new(idx, task.id), task)),
            }
        }
        log::debug!("searching complete");

        if !show_dropped {
            candidates.retain(|(_, t)| t.status != DROPPED_STATUS);
        }
        candidates
    }

    fn select_interactive(&self, needle: &str, show_dropped: bool) -> Option<Loc> {
        let candidates: Vec<_> = self.find(needle, show_dropped).into_iter().collect();
        match candidates.as_slice() {
            [] => None,
            [one] => Some(one.0),
            many => {
                println!("Select ID:");
                print_tasks(many.iter().map(|(_, x)| *x), None);
                let id: usize = read_line().ok()?.parse().ok()?;
                // Despite the fact this id may exist, we force user to choose only
                // over the list we printed to prevent mistakes
                many.iter()
                    .find_map(|(loc, _)| if loc.id == id { Some(*loc) } else { None })
            }
        }
    }

    fn iter(&self) -> impl Iterator<Item = &Task> {
        self.inner.iter()
    }
}

fn print_tasks<'a>(tasks: impl Iterator<Item = &'a Task> + 'a, only_status: Option<String>) {
    let mut by_status: HashMap<_, Vec<_>> = HashMap::new();
    for task in tasks {
        by_status.entry(task.status.clone()).or_default().push(task);
    }

    let statuses = match only_status {
        Some(v) => vec![v],
        None => vec![TODO_STATUS.into(), DONE_STATUS.into()],
    };

    for status in statuses {
        if let Some(status_tasks) = by_status.get(&status) {
            println!("[{status}]:");
            for task in status_tasks {
                println!("{task}");
            }
        }
    }
}

fn is_similar_words(needles: &[&str], haystack: &[&str]) -> bool {
    debug_assert!(needles.iter().all(|w| w.to_lowercase() == *w));
    debug_assert!(haystack.iter().all(|w| w.to_lowercase() == *w));

    let mut weights = Vec::with_capacity(needles.len() + haystack.len());
    for needle_word in needles {
        for haystack_word in haystack {
            weights.push((
                jaro_winkler(needle_word, haystack_word),
                needle_word,
                haystack_word,
            ));
        }
    }
    weights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Less));
    weights.reverse();
    if let Some((sim, n, h)) = weights
        .iter()
        .find(|(x, needle, title)| x >= &0.999 && (needle.len() >= 3 || title.len() >= 3))
    {
        log::debug!("found 99.9%+ similar word: {sim} ({n} x {h})");
        return true;
    }
    let sum: f64 = weights.iter().take(needles.len()).map(|(x, _, _)| x).sum();
    #[allow(clippy::cast_precision_loss)]
    let count = (needles.len().saturating_sub(1) + 1) as f64;
    let avg = sum / count;
    if avg > 0.85 {
        log::debug!("average similarity is more than 85%: {avg}");
        return true;
    }
    false
}

#[derive(Debug, Copy, Clone)]
enum Candidate {
    No,
    ById,
    SubsetOfTitle,
    SimilarTitle,
    SubsetOfComment,
    SimilarComment,
}

impl Candidate {
    fn check(needle: &str, task: &Task) -> Self {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return Candidate::No;
        }
        if let Ok(id) = needle.parse::<usize>() {
            if task.id == id {
                return Candidate::ById;
            }
        }

        let needle_words = needle.split_whitespace().collect::<Vec<_>>();
        let title = task.title.to_lowercase();
        log::debug!("checking title '{title}'");
        if title.contains_all(&needle_words) {
            return Candidate::SubsetOfTitle;
        }

        if is_similar_words(&needle_words, &title.split_whitespace().collect::<Vec<_>>()) {
            return Candidate::SimilarTitle;
        }

        if !task.comments.is_empty() {
            let comment = task.comments.to_lowercase();
            log::debug!("checking comment '{comment}'");
            if comment.contains_all(&needle_words) {
                return Candidate::SubsetOfComment;
            }
            if is_similar_words(
                &needle_words,
                &comment.split_whitespace().collect::<Vec<_>>(),
            ) {
                return Candidate::SimilarComment;
            }
        }

        Candidate::No
    }
}

macro_rules! print_not_found {
    () => {
        println!("Not found")
    };
}

#[allow(clippy::too_many_lines)]
fn main() -> io::Result<()> {
    env_logger::builder()
        .parse_default_env()
        .format_timestamp_micros()
        .init();
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
                .and_then(|loc| tasks.set_done_idx(loc.idx))
            {
                None => print_not_found!(),
                Some(t) => println!("Done: {t}"),
            }
            tasks.save()?;
        }
        Command::Todo { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task, true)
                .and_then(|loc| tasks.set_todo_idx(loc.idx))
            {
                None => print_not_found!(),
                Some(t) => println!("TODO: {t}"),
            }
            tasks.save()?;
        }
        Command::Drop { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task, false)
                .and_then(|loc| tasks.set_dropped_idx(loc.idx))
            {
                None => print_not_found!(),
                Some(t) => println!("Dropped: {t}"),
            }
            tasks.save()?;
        }
        Command::External(args) => {
            let mut tasks = Tasks::load_default()?;
            let title = args.join(" ");
            let loc = tasks.todo(title, String::new());
            tasks.save()?;
            let task = tasks.find_idx(loc.idx).unwrap();
            println!("Task has been created: {task}");
        }
        Command::Find { task } => {
            let task = task.join(" ");
            let tasks = Tasks::load_default()?;
            let matched = tasks.find(&task, false).into_iter().map(|(_, t)| t);
            print_tasks(matched, None);
        }
        Command::Detail { task } => {
            let task = task.join(" ");
            let tasks = Tasks::load_default()?;

            match tasks
                .select_interactive(&task, true)
                .and_then(|loc| tasks.find_idx(loc.idx))
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
                .and_then(|loc| tasks.find_idx_mut(loc.idx))
            {
                None => print_not_found!(),
                Some(task) => {
                    println!("Comment for {task}:");
                    let comment = read_lines()?;
                    if !comment.is_empty() {
                        task.add_comment(&comment);
                    }
                }
            }

            tasks.save()?;
        }
        Command::Rename { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task, false)
                .and_then(|loc| tasks.find_idx_mut(loc.idx))
            {
                None => print_not_found!(),
                Some(task) => {
                    println!("New name for {task}:");
                    let new_title = read_line()?;
                    task.change_title(new_title);
                }
            }
            tasks.save()?;
        }
        Command::RemoveDropped => {
            println!("Are you sure? [y/N]");
            let confirmation = read_line()?.to_lowercase();
            if ["y", "yes"].contains(&&*confirmation) {
                let mut tasks = Tasks::load_default()?;
                let removed_n = tasks.remove_dropped();
                tasks.save()?;
                if removed_n > 0 {
                    println!("{removed_n} dropped tasks were removed");
                } else {
                    println!("Nothing to remove");
                }
            }
        }
        Command::Where => {
            if let Some(path) = Tasks::default_path().to_str() {
                println!("{path}")
            }
        }
    }
    Ok(())
}
