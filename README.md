# TODO

`todo` is a dead simple command-line tool for managing a task list.
Tasks are stored in a CSV file.

# Features

* Adding tasks
* Moving tasks between "todo" and "done" groups
* Searching tasks
* Removing tasks
* Commenting tasks

That's all. Really. No deadlines, no linking, no epics. No features are also a feature.

## Installation

You need Rust toolchain to build `todo` from sources.
Follow [this](https://doc.rust-lang.org/cargo/getting-started/installation.html)
instructions if you don't have one.

```sh
cargo install --git https://github.com/deliro/todo
```

## Usage

Run the CLI with one of the available commands:

### List tasks

```sh
todo list
```

Aliases: `l`, `ls`.

or just

```sh 
todo
```

### Add a new task

```sh
todo <task description>
```

Example:

```sh
todo make a wish
```

This will create a task `make a wish`.

```sh
todo "watch https://www.youtube.com/watch?v=dQw4w9WgXcQ"
```

Quotes are needed when your title have some "special" characters for your shell

### Change task status

#### Mark task as "todo"

```sh
todo todo <task>
```

Alias: `t`.

#### Mark task as "done"

```sh
todo done <task>
```

### Deleting tasks

#### Soft delete (drop a task)

```sh
todo drop <task>
```

Aliases: `remove`, `delete`, `rm`.

#### Permanently remove all dropped tasks

```sh
todo remove-dropped
```

### Rename a task

```sh
todo rename <task>
```

### Find tasks

```sh
todo find <task>
```

### Show task details (comments)

```sh
todo detail <task>
```

Alias: `d`.

### Add a comment to a task

```sh
todo comment <task>
```

Alias: `c`.

This will open your `$EDITOR` if it's set or open the first found from the list:
`nvim`, `vim`, `vi`, `nano`

A comment can also be read from piped stdin:

```sh 
cat somefile.txt | todo comment 123
```

will add the whole `somefile.txt` to the task with ID=123

## Example usage

```sh
todo buy milk     # Creates a task "buy milk"
todo list         # Displays the task list
todo done buy milk  # Marks "buy milk" as done
todo drop buy milk  # Deletes "buy milk"
```

## How tasks are found

The search is flexible. If you type `todo comment 123`, it will search for a task
with ID=123, if no task with this ID was found, `todo` will search for tasks that:

1. Have `123` in their title
2. Have `123` in their comments
3. Have a similar word in their title or comments. Similar words are found using Jaro-Winkler similarity.

If multiple candidates are found, you'll be prompted for the certain ID of the task you're looking for

### Example:

```sh 
todo buy milk
todo buy beer
todo learn how to make beer
```

```sh
todo find mlik  # a mistake

[Todo]:
1. buy milk
```

```sh 
todo comment beer

Select ID:
[Todo]:
2. buy beer
3. learn how to make beer
```

```sh
todo detail learning

Title: learn how to make beer
ID: 3
Status: Todo
----- comments -----
where to buy hops?
--------------------
created at: 2025-03-23T21:41:07.640567+03:00
updated at: 2025-03-23T21:43:05.604565+03:00
```

### Backing up your tasks

It's a good idea to back up your tasks on a regular basis. Git is a perfect tool for
that.

Do the following steps:

1. Create an empty git repository somewhere (private GitHub repository for example)
2. `cd $(dirname $(todo w))`
3. `git init`
4. `git remote add origin git@github.com:your-username/repo-name.git`
5. `git add tasks.csv`
6. `git commit -m "initial"`
7. `git push -u origin master`
8. run `which todo` and remember the path
9. `crontab -e`
10. ```sh
    MAILTO=""
    0 * * * * cd $(dirname $(/the/path/from/step-8 w)) && git diff --quiet || (git add tasks.csv && git commit -m "
    Auto-commit $(date +\%Y-\%m-\%d\ \%H:\%M:\%S)" && git push)
    ```
11. save the crontab file (`ESC-Z-Z` in case you're in vim/nvim/vi)

Now your tasks are backed up every hour if changes were made.

## License

This project is distributed under the MIT License.
