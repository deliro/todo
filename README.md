# TODO

`todo` is a dead simple command-line tool for managing a task list.
Tasks are stored in a CSV file.

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

### Add a new task

```sh
todo <task description>
```

Example:

```sh
todo make a wish
```

This will create a task `make a wish`.

### Change task status

#### Mark task as "todo" from "drop" or "done" statuses

```sh
todo recover <task>
```

Alias: `todo`

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

## Example usage

```sh
todo buy milk     # Creates a task "buy milk"
todo list         # Displays the task list
todo done buy milk  # Marks "buy milk" as done
todo drop buy milk  # Deletes "buy milk"
```

## License

This project is distributed under the [MIT License](LICENSE).
