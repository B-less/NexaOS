//! `nexa-pack new`: starter projects that build and package as they are.

use std::fs;
use std::path::Path;

const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <rect x="8" y="8" width="112" height="112" rx="26" fill="#2a2a2a"/>
  <polygon points="64,24 98,44 98,84 64,104 30,84 30,44" fill="none" stroke="#7aa2f7" stroke-width="7"/>
</svg>
"##;

fn c_files(id: &str, name: &str) -> Vec<(String, String)> {
    let toml = format!(r#"[package]
id = "{id}"
name = "{name}"
version = "0.1.0"
description = "A NexaOS app written in C"
categories = "Utility;"
icon = "icon.svg"

[build]
command = "make"
binary = "build/{id}"

[requires]
depends = ["gtk+3.0"]
permissions = []
"#);
    let makefile = format!(r#"# Hardened build: stack protector, fortify, position-independent, full RELRO.
CFLAGS ?= -O2 -g0
CFLAGS += -std=c11 -Wall -Wextra -Werror -fstack-protector-strong \
          -D_FORTIFY_SOURCE=2 -fPIE $(shell pkg-config --cflags gtk+-3.0)
LDFLAGS += -pie -Wl,-z,relro,-z,now
LDLIBS += $(shell pkg-config --libs gtk+-3.0)

build/{id}: src/main.c | build
	$(CC) $(CFLAGS) -o $@ $< $(LDFLAGS) $(LDLIBS)

build:
	mkdir -p build

clean:
	rm -rf build dist
.PHONY: clean
"#);
    let main_c = format!(r#"/* {name}: a NexaOS app in C (GTK 3). Build: nexa-pack build */
#include <gtk/gtk.h>

static void on_click(GtkButton *button, gpointer label)
{{
    static int clicks;
    char text[64];
    clicks++;
    g_snprintf(text, sizeof text, "Clicked %d time%s", clicks, clicks == 1 ? "" : "s");
    gtk_label_set_text(GTK_LABEL(label), text);
    (void)button;
}}

int main(int argc, char **argv)
{{
    gtk_init(&argc, &argv);
    g_set_prgname("{id}");

    GtkWidget *win = gtk_window_new(GTK_WINDOW_TOPLEVEL);
    gtk_window_set_default_size(GTK_WINDOW(win), 420, 240);
    GtkWidget *bar = gtk_header_bar_new();
    gtk_header_bar_set_title(GTK_HEADER_BAR(bar), "{name}");
    gtk_header_bar_set_show_close_button(GTK_HEADER_BAR(bar), TRUE);
    gtk_window_set_titlebar(GTK_WINDOW(win), bar);

    GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 12);
    gtk_container_set_border_width(GTK_CONTAINER(box), 24);
    GtkWidget *label = gtk_label_new("Hello from NexaOS");
    GtkWidget *button = gtk_button_new_with_label("Click me");
    g_signal_connect(button, "clicked", G_CALLBACK(on_click), label);
    gtk_box_pack_start(GTK_BOX(box), label, TRUE, TRUE, 0);
    gtk_box_pack_start(GTK_BOX(box), button, FALSE, FALSE, 0);
    gtk_container_add(GTK_CONTAINER(win), box);

    g_signal_connect(win, "destroy", G_CALLBACK(gtk_main_quit), NULL);
    gtk_widget_show_all(win);
    gtk_main();
    return 0;
}}
"#);
    vec![
        ("nexa.toml".into(), toml),
        ("Makefile".into(), makefile),
        ("src/main.c".into(), main_c),
        ("icon.svg".into(), ICON.into()),
        (".gitignore".into(), "build/\ndist/\n".into()),
    ]
}

fn rust_files(id: &str, name: &str) -> Vec<(String, String)> {
    let crate_name = id.replace('-', "_");
    let toml = format!(r#"[package]
id = "{id}"
name = "{name}"
version = "0.1.0"
description = "A NexaOS tool written in Rust"
categories = "System;"
icon = "icon.svg"
terminal = true

[build]
command = "cargo build --release"
binary = "target/release/{crate_name}"

[requires]
depends = []
permissions = []
"#);
    let cargo = format!(r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"

[dependencies]

[profile.release]
opt-level = "s"
lto = true
strip = true
panic = "abort"
"#);
    let main_rs = format!(r#"//! {name}: a NexaOS tool in Rust. Build: nexa-pack build
use std::fs;

fn read(path: &str) -> String {{
    fs::read_to_string(path).unwrap_or_default()
}}

fn main() {{
    let uptime: f64 = read("/proc/uptime").split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let mem_kb = |key: &str| -> u64 {{
        read("/proc/meminfo").lines().find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse().ok()).unwrap_or(0)
    }};
    let (total, avail) = (mem_kb("MemTotal:"), mem_kb("MemAvailable:"));
    println!("Hello from NexaOS ({name})");
    println!("  up for {{:.1}} hours", uptime / 3600.0);
    println!("  memory: {{}} of {{}} MB in use", (total - avail) / 1024, total / 1024);
}}
"#);
    vec![
        ("nexa.toml".into(), toml),
        ("Cargo.toml".into(), cargo),
        ("src/main.rs".into(), main_rs),
        ("icon.svg".into(), ICON.into()),
        (".gitignore".into(), "target/\ndist/\n".into()),
    ]
}

pub fn create(dir: &Path, id: &str, name: &str, lang: &str) -> Result<(), String> {
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    let files = match lang {
        "c" => c_files(id, name),
        "rust" => rust_files(id, name),
        other => return Err(format!("unknown language '{other}' (c or rust)")),
    };
    for (rel, body) in files {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}
