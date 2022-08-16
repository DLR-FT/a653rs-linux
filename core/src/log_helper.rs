use std::io::{Result, Write};

use log::{Level, Record};
use pretty_env_logger::env_logger::fmt::{Color, Formatter};

use crate::partition::NAME_ENV;

//pub fn colored_level<'a>(
//    style: &'a mut pretty_env_logger::env_logger::fmt::Style,
//    level: Level,
//) -> pretty_env_logger::env_logger::fmt::StyledValue<'a, &'static str> {
//    match level {
//        Level::Trace => style.set_color(Color::Magenta).value("TRACE"),
//        Level::Debug => style.set_color(Color::Blue).value("DEBUG"),
//        Level::Info => style.set_color(Color::Green).value("INFO "),
//        Level::Warn => style.set_color(Color::Yellow).value("WARN "),
//        Level::Error => style.set_color(Color::Red).value("ERROR"),
//    }
//}
//
//pub fn format(f: &mut Formatter, record: &Record) -> Result<()> {
//    let mut style = f.style();
//    let name = std::env::var(NAME_ENV).unwrap_or_default();
//    let name = style.set_bold(true).value(name);
//
//    let mut style = f.style();
//    let level = colored_level(&mut style, record.level());
//
//    let mut style = f.style();
//    let target = record.target();
//    let target = style.set_bold(true).value(target);
//
//    let msg = record.args();
//
//    writeln!(f, "{level} {name}/{target} > {msg}")
//}
