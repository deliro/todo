use clap::{Parser, Subcommand};
use csv::{ReaderBuilder, WriterBuilder};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::env::home_dir;
use std::fs::File;
use std::io;
use std::io::{BufReader, stdin};
use std::path::PathBuf;
use std::time::SystemTime;
use strsim::jaro_winkler;

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
    /// Список задач
    List {
        status: Option<String>,
    },
    /// Изменить статус задачи
    Todo {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        task: Vec<String>,
    },
    /// Отметить задачу как выполненную
    Done {
        task: Vec<String>,
    },

    Drop {
        task: Vec<String>,
    },

    Find {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        task: Vec<String>,
    },
    Comment {
        task: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        comment: Vec<String>,
    },
    #[clap(external_subcommand)]
    External(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Task {
    id: usize,
    status: String,
    title: String,
    // TODO: chrono
    created_at: SystemTime,
    updated_at: SystemTime,
    comments: String,
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
        let mut file = home_dir().expect("cannot determine home directory");
        file.push(".todo");
        file.push("tasks.csv");
        Self::load(file)
    }

    fn load(filename: PathBuf) -> io::Result<Self> {
        // FIXME: descriptor errors
        let file = File::open(&filename).or_else(|_| File::create(&filename))?;
        let reader = BufReader::new(file);
        let mut rdr = ReaderBuilder::new().has_headers(false).from_reader(reader);
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
        task.status = status;
        Some(task)
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
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
        };
        self.inner.push(task);
        self.inner.iter().last().unwrap()
    }

    fn save(&self) -> io::Result<()> {
        let file = File::create(&self.filename)?;
        let mut wtr = WriterBuilder::new().has_headers(false).from_writer(file);
        for record in &self.inner {
            wtr.serialize(record)?;
        }

        wtr.flush()?;
        Ok(())
    }

    fn find_id(&self, id: usize) -> Option<&Task> {
        self.inner.iter().find(|t| t.id == id)
    }

    fn find(&self, name_or_id: &str) -> Vec<&Task> {
        self.find_inner(name_or_id, false)
    }

    fn find_inner(&self, name_or_id: &str, even_deleted: bool) -> Vec<&Task> {
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

        if !even_deleted {
            candidates.retain(|t| t.status != DROPPED_STATUS)
        }
        candidates.dedup_by(|t, t2| t.id == t2.id);
        candidates
    }

    fn select_interactive(&self, needle: &str) -> Option<usize> {
        let ids: Vec<_> = self.find(needle).iter().map(|t| t.id).collect();
        match ids.as_slice() {
            [] => None,
            [id] => Some(*id),
            many => {
                println!("Укажите ID:");
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

const TODO_STATUS: &str = "todo";
const DONE_STATUS: &str = "done";
const DROPPED_STATUS: &str = "drop";

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
                println!("{}. {}", task.id, task.title)
            }
        }
    }
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
                .select_interactive(&task)
                .and_then(|id| tasks.set_done(id))
            {
                None => println!("Задача не найдена"),
                Some(t) => println!("Задача готова: {}. {}", t.id, t.title),
            }
            tasks.save()?;
        }
        Command::Todo { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task)
                .and_then(|id| tasks.set_todo(id))
            {
                None => println!("Задача не найдена"),
                Some(t) => println!("Задача в TODO: {}. {}", t.id, t.title),
            }
            tasks.save()?;
        }
        Command::Drop { task } => {
            let task = task.join(" ");
            let mut tasks = Tasks::load_default()?;
            match tasks
                .select_interactive(&task)
                .and_then(|id| tasks.set_dropped(id))
            {
                None => println!("Задача не найдена"),
                Some(t) => println!("Задача удалена: {}. {}", t.id, t.title),
            }
            tasks.save()?;
        }
        Command::External(args) => {
            let mut tasks = Tasks::load_default()?;
            let title = args.join(" ");
            let id = tasks.todo(title, "".into()).id;
            tasks.save()?;
            let task = tasks.find_id(id).unwrap();
            println!("Задача #{} добавлена: {}", task.id, task.title);
        }
        Command::Find { task } => {
            let task = task.join(" ");
            let tasks = Tasks::load_default()?;
            let matched = tasks.find(&task);
            print_tasks(matched.into_iter(), None)
        }
        _ => todo!(),
    }
    Ok(())
}
