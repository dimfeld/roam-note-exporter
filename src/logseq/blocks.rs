use std::io::BufRead;

use anyhow::anyhow;
use fxhash::FxHashMap;
use nom::{
    branch::alt,
    bytes::complete::{tag, take_while},
    combinator::{map, opt},
    multi::many0,
    sequence::{terminated, tuple},
    IResult,
};
use smallvec::SmallVec;

use crate::graph::Block;

use super::{attrs::parse_attr_line, LinesIterator};

#[derive(Debug, PartialEq, Eq)]
struct Line<'a> {
    contents: &'a str,
    indent: u32,
    header: u32,
    new_block: bool,
    attr_name: String,
    attr_values: Vec<String>,
}

pub struct LogseqRawBlock {
    id: String,
    header_level: usize,
    contents: String,
    indent: u32,
}

pub fn parse_raw_blocks(
    lines: &mut LinesIterator<impl BufRead>,
) -> Result<Vec<LogseqRawBlock>, anyhow::Error> {
    let mut blocks = Vec::new();

    loop {
        match read_raw_block(lines)? {
            RawBlockOutput::Done => break,
            RawBlockOutput::Empty => {}
            RawBlockOutput::Block(block) => {
                blocks.push(block);
            }
        }
    }

    Ok(blocks)
}

enum RawBlockOutput {
    Done,
    Empty,
    Block(LogseqRawBlock),
}

// Take the first line separately just because that's how the header parser returns it.
fn read_raw_block(
    lines: &mut LinesIterator<impl BufRead>,
) -> Result<RawBlockOutput, anyhow::Error> {
    // Most blocks will just be one or two lines
    let mut line_contents: SmallVec<[String; 2]> = SmallVec::new();
    let mut indent = 0;
    let mut id = String::new();

    loop {
        let line_read = lines.next();
        if let Some(line) = line_read {
            let line = line?;
            if line.is_empty() {
                continue;
            }

            let parsed = evaluate_line(line.as_str())?;
            match parsed {
                None => break,
                Some(mut parsed) => {
                    if line_contents.is_empty() {
                        assert!(parsed.new_block);
                        indent = parsed.indent;
                    } else if parsed.new_block {
                        // Done with this block.
                        lines.put_back(Ok(line));
                        break;
                    }

                    if parsed.attr_name == "id" {
                        // Record the "id" attribute but omit it from the contents since it isn't added
                        // by the user.
                        id = parsed.attr_values.pop().unwrap_or_default();
                    } else {
                        line_contents.push(parsed.contents.to_string());
                    }
                }
            }
        } else {
            return Ok(RawBlockOutput::Done);
        }
    }

    if line_contents.is_empty() {
        return Ok(RawBlockOutput::Empty);
    }

    let block_contents = LogseqRawBlock {
        id,
        header_level: 0,
        contents: line_contents.join("\n"),
        indent,
    };

    Ok(RawBlockOutput::Block(block_contents))
}

fn count_repeated_char(input: &str, match_char: char) -> IResult<&str, u32> {
    map(take_while(|c| c == match_char), |result: &str| {
        result.chars().count() as u32
    })(input)
}

fn evaluate_line(line: &str) -> Result<Option<Line<'_>>, anyhow::Error> {
    if line.is_empty() {
        return Ok(None);
    }

    let (rest, (indent, (dash, header))) = tuple((
        |input| count_repeated_char(input, '\t'),
        alt((
            map(
                tuple((
                    tag("- "),
                    opt(terminated(
                        |input| count_repeated_char(input, '#'),
                        tag(" "),
                    )),
                )),
                |(_, header_level)| (true, header_level.unwrap_or(0)),
            ),
            map(tag("  "), |_| (false, 0)),
        )),
    ))(line)
    .map_err(|e| anyhow!("{}", e))?;

    let (attr_name, attr_values) = match parse_attr_line("::", rest) {
        Ok(Some(v)) => v,
        _ => (String::new(), Vec::new()),
    };

    Ok(Some(Line {
        contents: rest,
        indent,
        new_block: dash,
        header,
        attr_name,
        attr_values,
    }))
}

#[cfg(test)]
mod test {
    mod evaluate_line {
        use super::super::{evaluate_line, Line};

        #[test]
        fn empty_line() {
            let input = "";
            let result = evaluate_line(input).unwrap();
            assert!(result.is_none(), "Should be none: {:?}", result);
        }

        #[test]
        fn no_indent_same_block() {
            let input = "  abc";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "abc",
                    indent: 0,
                    header: 0,
                    new_block: false,
                    attr_name: String::new(),
                    attr_values: Vec::new(),
                }
            );
        }

        #[test]
        fn extra_spaces_same_block() {
            let input = "    abc";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "  abc",
                    indent: 0,
                    header: 0,
                    new_block: false,
                    attr_name: String::new(),
                    attr_values: Vec::new(),
                }
            );
        }

        #[test]
        fn indent_same_block() {
            let input = "\t\t  abc";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "abc",
                    indent: 2,
                    header: 0,
                    new_block: false,
                    attr_name: String::new(),
                    attr_values: Vec::new(),
                }
            );
        }

        #[test]
        fn no_indent_new_block() {
            let input = "- abc";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "abc",
                    indent: 0,
                    header: 0,
                    new_block: true,
                    attr_name: String::new(),
                    attr_values: Vec::new(),
                }
            );
        }

        #[test]
        fn indent_new_block() {
            let input = "\t\t- abc";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "abc",
                    indent: 2,
                    header: 0,
                    new_block: true,
                    attr_name: String::new(),
                    attr_values: Vec::new(),
                }
            );
        }

        #[test]
        fn header1() {
            let input = "\t\t- # abc";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "abc",
                    indent: 2,
                    header: 1,
                    new_block: true,
                    attr_name: String::new(),
                    attr_values: Vec::new(),
                }
            );
        }

        #[test]
        fn header3() {
            let input = "\t\t- ### abc";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "abc",
                    indent: 2,
                    header: 3,
                    new_block: true,
                    attr_name: String::new(),
                    attr_values: Vec::new(),
                }
            );
        }

        #[test]
        fn hashes_on_second_line() {
            let input = "\t\t  ### abc";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "### abc",
                    indent: 2,
                    header: 0,
                    new_block: false,
                    attr_name: String::new(),
                    attr_values: Vec::new(),
                }
            );
        }

        #[test]
        fn new_block_attr_line() {
            let input = "\t\t- abc:: def";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "abc:: def",
                    indent: 2,
                    header: 0,
                    new_block: true,
                    attr_name: String::from("abc"),
                    attr_values: vec![String::from("def")]
                }
            );
        }

        #[test]
        fn same_block_attr_line() {
            let input = "\t\t  abc:: def";
            assert_eq!(
                evaluate_line(input).unwrap().unwrap(),
                Line {
                    contents: "abc:: def",
                    indent: 2,
                    header: 0,
                    new_block: false,
                    attr_name: String::from("abc"),
                    attr_values: vec![String::from("def")]
                }
            );
        }
    }
}
