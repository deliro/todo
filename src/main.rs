mod filter_parser;

use crate::filter_parser::Attr;
use atty::Stream;
use chrono::{Local, Utc};
use clap::{Parser, Subcommand};
use csv::{ReaderBuilder, WriterBuilder};
use homedir::my_home;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::cmp::{Ordering, PartialEq};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write, stdin};
use std::path::PathBuf;
use std::process::Command as Cmd;
use std::str::FromStr;
use std::{env, fmt, fs, io};
use strsim::jaro_winkler;

static TRANSLIT_MAP: Lazy<HashMap<char, char>> = Lazy::new(|| {
    const ENG: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,./";
    const RUS: &str = "йцукенгшщзхъфывапролджэячсмитьбю.";

    ENG.chars().zip(RUS.chars()).collect()
});

fn translate(input: &str) -> String {
    input
        .chars()
        .map(|c| TRANSLIT_MAP.get(&c).copied().unwrap_or(c))
        .collect()
}

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
            let mut tmp_file = tempfile::Builder::new().suffix(".md").tempfile()?;
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
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print `todo` and `done` tasks lists
    #[clap(visible_aliases = &["l", "ls"])]
    List { status: Option<String> },
    /// Change status to `todo`
    #[clap(visible_aliases = &["t", "recover"])]
    Todo { task: Vec<String> },
    /// Change status to `done`
    #[clap(visible_alias = "dn")]
    Done { task: Vec<String> },
    /// Remove a task. If the task in `todo` or `done` status, soft-deletes it
    /// (set `drop` status). If the task is already in `drop` status, physically
    /// removes it.
    #[clap(visible_aliases = &["remove", "delete", "rm"])]
    Drop { task: Vec<String> },
    /// Rename a task
    #[clap(visible_alias = "r")]
    Rename { task: Vec<String> },
    /// Find tasks (including `drop` status)
    #[clap(visible_alias = "f")]
    Find { task: Vec<String> },
    /// Show a task's details and comments
    #[clap(visible_alias = "d")]
    Detail { task: Vec<String> },
    /// Add a comment to a task
    #[clap(visible_alias = "c")]
    Comment { task: Vec<String> },
    /// Create new task in `done` status
    Log { task: Vec<String> },
    /// Physically remove all tasks in `drop` status
    RemoveDropped,
    /// Soft-delete all done tasks (set `drop` status)
    DropDone,
    /// Print the tasks file path
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
    const ALL: &'static [Self] = &[Self::Drop, Self::Done, Self::Todo];
    const VISIBLE: &'static [Self] = &[Self::Done, Self::Todo];

    fn is_visible(self) -> bool {
        match self {
            Status::Todo | Status::Done => true,
            Status::Drop => false,
        }
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
        write!(f, "{}. {}", self.id, self.title)?;
        if !self.comments.trim().is_empty() {
            write!(f, " [*]")?;
        }
        Ok(())
    }
}

impl Task {
    fn details(&self) -> Result<String, fmt::Error> {
        use std::fmt::Write;

        let mut buf = String::with_capacity(128);
        writeln!(buf, "Title: {}", self.title)?;
        writeln!(buf, "ID: {}", self.id)?;
        writeln!(buf, "Status: {}", self.status)?;
        writeln!(
            buf,
            "created at: {:?}",
            self.created_at.with_timezone(&Local)
        )?;
        writeln!(
            buf,
            "updated at: {:?}",
            self.updated_at.with_timezone(&Local)
        )?;
        if !self.comments.is_empty() {
            writeln!(buf, "{}", termimad::term_text("------------------------"))?;
            writeln!(buf, "{}", termimad::term_text(&self.comments))?;
        }
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
        if let Some((_, value)) =
            env::vars().find(|(key, value)| key == "TASKS_FILE" && !value.trim().is_empty())
        {
            log::debug!("TASKS_FILE was found: {value:?}");
            return value.trim().into();
        }
        let mut file = my_home()
            .transpose()
            .unwrap()
            .expect("cannot determine home directory");
        file.push(".todo");
        file.push("tasks.csv");
        file
    }
    fn load_default() -> io::Result<Self> {
        Self::load(Self::default_path())
    }

    fn load(filename: PathBuf) -> io::Result<Self> {
        log::info!("loading tasks from {filename:?}");
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

    fn drop_done(&mut self) -> usize {
        let mut dropped = 0;
        self.inner.iter_mut().for_each(|task| {
            if task.status == Status::Done {
                task.set_status(Status::Drop);
                dropped += 1
            }
        });
        dropped
    }

    fn remove(&mut self, idx: Idx) -> Option<Task> {
        let idx = idx.into();
        if idx < self.inner.len() {
            Some(self.inner.remove(idx))
        } else {
            None
        }
    }

    fn next_loc(&self) -> Loc {
        // We might have assumed the last vec element is the latest hence has the
        // greatest ID, but the tasks file may be externally shuffled so seq scan
        // is the only option.
        let next_id = self.inner.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        let next_idx = self.inner.len();
        Loc::new(next_idx, next_id)
    }

    fn add(&mut self, title: String, status: Status) -> Loc {
        let loc = self.next_loc();
        debug_assert_eq!(loc.idx, self.inner.len().into());
        let task = Task {
            id: loc.id,
            title,
            comments: String::new(),
            status,
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
        log::info!("file saved");
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

    fn find(&self, needle: &str, show_dropped: bool, empty_show_all: bool) -> Vec<(Loc, &Task)> {
        let needle = needle.trim().to_lowercase();
        let mut candidates = vec![];
        if needle.is_empty() {
            return match empty_show_all {
                true => self
                    .iter()
                    .enumerate()
                    .map(|(idx, task)| (Loc::new(idx, task.id), task))
                    .collect(),
                false => candidates,
            };
        }
        log::debug!("searching candidates for '{needle}'");
        for (idx, task) in self.iter().enumerate() {
            let candidate = Candidate::check(&needle, task)
                .or_else(|| Candidate::check(&translate(&needle), task));
            log::debug!("candidate '{task}' result is {candidate:?}");
            if let Some(candidate) = candidate {
                match candidate {
                    Candidate::ById if show_dropped || task.status.is_visible() => {
                        log::debug!("searching stopped because ID was found");
                        return vec![(Loc::new(idx, task.id), task)];
                    }
                    _ => candidates.push((Loc::new(idx, task.id), task)),
                }
            }
        }
        log::debug!("searching complete");

        if !show_dropped {
            candidates.retain(|(_, t)| t.status.is_visible());
        }
        candidates
    }

    fn select_interactive(&self, needle: &str, show_dropped: bool) -> Option<Loc> {
        let candidates: Vec<_> = self.find(needle, show_dropped, false).into_iter().collect();
        match candidates.as_slice() {
            [] => None,
            [one] => Some(one.0),
            many => {
                println!("Select ID:");
                let tasks = many.iter().map(|(_, x)| *x);
                match show_dropped {
                    true => print_all_tasks(tasks),
                    false => print_visible_tasks(tasks),
                };
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
    print_only_status_tasks(tasks, Status::VISIBLE)
}

fn print_all_tasks<'a>(tasks: impl Iterator<Item = &'a Task> + 'a) {
    print_only_status_tasks(tasks, Status::ALL)
}

fn print_only_status_tasks<'a>(
    tasks: impl Iterator<Item = &'a Task> + 'a,
    only_statuses: &[Status],
) {
    let mut by_status: HashMap<_, Vec<_>> = HashMap::new();
    for task in tasks {
        by_status.entry(&task.status).or_default().push(task);
    }
    for status in only_statuses {
        if let Some(status_tasks) = by_status.get(status) {
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
    ById,
    SubsetOfTitle,
    SimilarTitle,
    SubsetOfComment,
    SimilarComment,
}

impl Candidate {
    fn check(needle: &str, task: &Task) -> Option<Self> {
        debug_assert_eq!(needle, needle.trim().to_lowercase());
        log::debug!("checking needle '{needle}' against task {task}");
        if let Ok(id) = needle.parse::<usize>() {
            if task.id == id {
                return Some(Candidate::ById);
            }
        }

        let needle_words = needle.split_whitespace().collect::<Vec<_>>();
        let title = task.title.to_lowercase();
        if title.contains_all(&needle_words) {
            return Some(Candidate::SubsetOfTitle);
        }

        if is_similar_words(&needle_words, &title.split_whitespace().collect::<Vec<_>>()) {
            return Some(Candidate::SimilarTitle);
        }

        if !task.comments.is_empty() {
            let comment = task.comments.to_lowercase();
            if comment.contains_all(&needle_words) {
                return Some(Candidate::SubsetOfComment);
            }
            if is_similar_words(
                &needle_words,
                &comment.split_whitespace().collect::<Vec<_>>(),
            ) {
                return Some(Candidate::SimilarComment);
            }
        }

        None
    }
}

macro_rules! print_not_found {
    () => {
        println!("Not found")
    };
}

fn confirm() -> bool {
    println!("Are you sure? [y/N]");
    read_line().is_ok_and(|v| ["y", "yes"].contains(&v.to_lowercase().trim()))
}

#[allow(clippy::too_many_lines)]
fn main() -> io::Result<()> {
    env_logger::builder()
        .parse_default_env()
        .format_timestamp_micros()
        .init();
    let cli = TodoCli::parse();
    match cli.command {
        Some(Command::List { status }) => {
            let tasks = Tasks::load_default()?;
            match status {
                None => print_visible_tasks(tasks.iter()),
                Some(str_status) => match str_status.parse::<Status>() {
                    Ok(only_status) => print_only_status_tasks(tasks.iter(), &[only_status]),
                    Err(_) => {
                        log::debug!("Unknown status {str_status}");
                        print_visible_tasks(tasks.iter());
                    }
                },
            }
        }
        Some(Command::Done { task }) => {
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
        Some(Command::Todo { task }) => {
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
        Some(Command::Drop { task }) => {
            enum Method {
                Drop,
                Remove,
            }

            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task, true)
                .and_then(|loc| {
                    tasks.find_idx(loc.idx).map(|task| match task.status {
                        Status::Drop => (loc, Method::Remove),
                        _ => (loc, Method::Drop),
                    })
                })
                .and_then(|(loc, method)| match method {
                    Method::Drop => tasks.set_dropped_idx(loc.idx).cloned().map(|x| (method, x)),
                    Method::Remove => confirm()
                        .then(|| tasks.remove(loc.idx))
                        .flatten()
                        .map(|x| (method, x)),
                }) {
                None => print_not_found!(),
                Some((Method::Drop, t)) => println!("Dropped: {t}"),
                Some((Method::Remove, t)) => println!("Removed: {t}"),
            }

            tasks.save()?;
        }
        Some(Command::Find { task }) => {
            let tasks = Tasks::load_default()?;
            let task = task.join(" ").to_lowercase();
            let mut needle = task.as_str();
            let mut filter = None;
            if let Ok((tail, (attr, range))) = filter_parser::attr_and_range(&task) {
                needle = tail.trim();
                filter = Some((attr, range));
            }
            log::info!("filter is {filter:?}");
            let matched = tasks
                .find(needle, true, filter.is_some())
                .into_iter()
                .map(|(_, t)| t)
                .filter(|t| match &filter {
                    None => true,
                    Some((attr, range)) => match attr {
                        Attr::Updated => range.contains(&t.updated_at.date_naive()),
                        Attr::Created => range.contains(&t.created_at.date_naive()),
                    },
                });

            print_all_tasks(matched);
        }
        Some(Command::Detail { task }) => {
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
        Some(Command::Comment { task }) => {
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
        Some(Command::Rename { task }) => {
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
        Some(Command::RemoveDropped) => {
            if confirm() {
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
        Some(Command::Where) => {
            if let Some(path) = Tasks::default_path().to_str() {
                println!("{path}");
            }
        }
        Some(Command::DropDone) => {
            if confirm() {
                let mut tasks = Tasks::load_default()?;
                let dropped = tasks.drop_done();
                if dropped > 0 {
                    println!("{dropped} done tasks were dropped")
                } else {
                    println!("Nothing to drop")
                }
                tasks.save()?
            }
        }
        Some(Command::External(task)) => add_task(task.join(" "), Status::Todo)?,
        Some(Command::Log { task }) => add_task(task.join(" "), Status::Done)?,
        None => {
            let tasks = Tasks::load_default()?;
            print_only_status_tasks(tasks.iter(), &[Status::Todo])
        }
    }
    Ok(())
}

fn add_task(title: String, status: Status) -> io::Result<()> {
    let mut tasks = Tasks::load_default()?;
    let loc = tasks.add(title, status);
    tasks.save()?;
    let task = tasks.find_idx(loc.idx).unwrap();
    println!("Task has been created: {task}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translate() {
        assert_eq!(translate("ghbdtn"), "привет")
    }
}
