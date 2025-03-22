use atty::Stream;
use chrono::{Local, Utc};
use clap::{Parser, Subcommand};
use csv::{ReaderBuilder, WriterBuilder};
use serde::{Deserialize, Serialize};
use std::cmp::{Ordering, PartialEq};
use std::collections::HashMap;
#[allow(deprecated)]
use std::env::home_dir;
use std::fmt::{Display, Formatter};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write, stdin};
use std::path::PathBuf;
use std::process::Command as Cmd;
use std::str::FromStr;
use std::{env, fmt, fs, io};
use strsim::jaro_winkler;
use tempfile::NamedTempFile;

fn read_line() -> io::Result<String> {
    let mut buf = vec![];
    let mut handle = stdin().lock();
    handle.read_until(b'\n', &mut buf)?;
    Ok(String::from_utf8_lossy(&buf).trim().to_string())
}

fn get_editor() -> Option<String> {
    env::var("EDITOR")
        .ok()
        .and_then(|x| x.not_empty())
        .or_else(|| env::var("VISUAL").ok().and_then(|x| x.not_empty()))
        .or_else(|| {
            ["nvim", "vim", "vi", "nano"]
                .iter()
                .find(|&&e| which::which(e).is_ok())
                .map(|s| s.to_string())
        })
}

enum Multiline {
    Append(String),
    Full(String),
}

fn read_multiline(initial: &str) -> io::Result<Multiline> {
    Ok(match (atty::is(Stream::Stdin), get_editor()) {
        (true, Some(editor)) => Multiline::Full({
            let mut tmp_file = NamedTempFile::new()?;
            write!(tmp_file, "{}", initial)?;
            let path = tmp_file.path();
            Cmd::new(editor).arg(path).status()?;
            fs::read_to_string(path)?
        }),
        (is_tty, _) => Multiline::Append({
            let mut buf = vec![];
            let mut handle = stdin().lock();
            if is_tty {
                // Fallback variant when no suitable editor was found.
                // Just read one line from stdin
                log::error!("no editor was found");
                handle.read_until(b'\n', &mut buf)?;
            } else {
                handle.read_to_end(&mut buf)?;
            }
            String::from_utf8_lossy(&buf).trim().to_string()
        }),
    })
}

trait StringExt {
    fn contains_all<T: AsRef<str>>(&self, i: impl IntoIterator<Item = T>) -> bool;
    fn not_empty(self) -> Option<Self>
    where
        Self: Sized;
}

impl<T> StringExt for T
where
    T: AsRef<str>,
{
    fn contains_all<Item: AsRef<str>>(&self, i: impl IntoIterator<Item = Item>) -> bool {
        i.into_iter().all(|x| self.as_ref().contains(x.as_ref()))
    }

    fn not_empty(self) -> Option<Self> {
        if self.as_ref().is_empty() {
            None
        } else {
            Some(self)
        }
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
    #[clap(visible_alias = "t")]
    Todo { task: Vec<String> },
    /// Change status to done
    #[clap(visible_alias = "dn")]
    Done { task: Vec<String> },
    /// Remove a task (soft-delete)
    #[clap(visible_aliases = &["remove", "delete", "rm"])]
    Drop { task: Vec<String> },
    /// Rename a task
    #[clap(visible_alias = "r")]
    Rename { task: Vec<String> },
    /// Find tasks
    #[clap(visible_alias = "f")]
    Find { task: Vec<String> },
    /// Show a task's details (comments)
    #[clap(visible_alias = "d")]
    Detail { task: Vec<String> },
    /// Add a comment to a task
    #[clap(visible_alias = "c")]
    Comment { task: Vec<String> },
    /// Physically remove all dropped tasks
    RemoveDropped,
    /// Prints the tasks file path
    #[clap(visible_alias = "w")]
    Where,
    /// Create new task
    #[clap(external_subcommand)]
    External(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Hash, Eq, Copy)]
#[serde(rename_all = "lowercase")]
enum Status {
    Todo,
    Done,
    Drop,
}

impl Status {
    fn is_visible(self) -> bool {
        match self {
            Status::Todo | Status::Done => true,
            Status::Drop => false,
        }
    }

    fn list_visible() -> [Self; 2] {
        [Self::Todo, Self::Done]
    }
}

impl FromStr for Status {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "todo" => Ok(Status::Todo),
            "done" => Ok(Status::Done),
            "drop" => Ok(Status::Drop),
            _ => Err(()),
        }
    }
}

impl Display for Status {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Task {
    id: usize,
    status: Status,
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

    fn add_comment(&mut self, comment: Multiline) {
        let old = self.comments.clone();
        match comment {
            Multiline::Append(comment) => {
                if !comment.is_empty() {
                    if !self.comments.is_empty() {
                        self.comments.push('\n');
                    }
                    self.comments.push_str(&comment);
                }
            }
            Multiline::Full(comment) => self.comments = comment,
        }

        if self.comments != old {
            self.updated_at = Utc::now();
        }
    }

    fn set_status(&mut self, status: Status) {
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

    fn set_status_idx(&mut self, idx: Idx, status: Status) -> Option<&Task> {
        let task = self.find_idx_mut(idx)?;
        task.set_status(status);
        Some(task)
    }

    fn set_done_idx(&mut self, idx: Idx) -> Option<&Task> {
        self.set_status_idx(idx, Status::Done)
    }

    fn set_todo_idx(&mut self, idx: Idx) -> Option<&Task> {
        self.set_status_idx(idx, Status::Todo)
    }

    fn set_dropped_idx(&mut self, idx: Idx) -> Option<&Task> {
        self.set_status_idx(idx, Status::Drop)
    }

    fn remove_dropped(&mut self) -> usize {
        let orig_len = self.inner.len();
        self.inner.retain(|t| t.status.is_visible());
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
            status: Status::Todo,
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
                Candidate::ById if show_dropped || task.status.is_visible() => {
                    log::debug!("searching stopped because ID was found");
                    return vec![(Loc::new(idx, task.id), task)];
                }
                Candidate::No => {}
                _ => candidates.push((Loc::new(idx, task.id), task)),
            }
        }
        log::debug!("searching complete");

        if !show_dropped {
            candidates.retain(|(_, t)| t.status.is_visible());
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
                print_visible_tasks(many.iter().map(|(_, x)| *x));
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

fn print_visible_tasks<'a>(tasks: impl Iterator<Item = &'a Task> + 'a) {
    let mut by_status: HashMap<_, Vec<_>> = HashMap::new();
    for task in tasks {
        by_status.entry(&task.status).or_default().push(task);
    }
    for status in Status::list_visible() {
        if let Some(status_tasks) = by_status.get(&status) {
            println!("[{status}]:");
            for task in status_tasks {
                println!("{task}");
            }
        }
    }
}

fn print_only_status_tasks<'a>(tasks: impl Iterator<Item = &'a Task> + 'a, only_status: Status) {
    println!("[{only_status}]:");
    for task in tasks {
        if task.status == only_status {
            println!("{task}");
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
            match status {
                None => print_visible_tasks(tasks.iter()),
                Some(str_status) => match str_status.parse::<Status>() {
                    Ok(only_status) => print_only_status_tasks(tasks.iter(), only_status),
                    Err(_) => {
                        log::debug!("Unknown status {str_status}");
                        print_visible_tasks(tasks.iter());
                    }
                },
            }
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
            print_visible_tasks(matched);
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
                    let comment = read_multiline(task.comments.as_str())?;
                    task.add_comment(comment);
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
                println!("{path}");
            }
        }
    }
    Ok(())
}
