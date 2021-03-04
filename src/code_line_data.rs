use tokei::CodeStats;

#[derive(Clone, Debug, PartialEq)]
pub struct CodeLineData {
    pub spaces: u32,
    pub tabs: u32,
    pub text: u32,
}

impl CodeLineData {
    fn new(line: &[u8]) -> Self {
        let mut spaces: u32 = 0;
        let mut tabs: u32 = 0;
        let mut text: Option<usize> = None;
        for ix in 0..line.len() {
            let c = line[ix];
            if c == b' ' {
                spaces += 1;
            } else if c == b'\t' {
                tabs += 1;
            } else {
                text = Some(
                    String::from_utf8_lossy(&line[ix..line.len()])
                        .trim()
                        .chars()
                        .count(),
                );
                break;
            }
        }

        CodeLineData {
            spaces,
            tabs,
            text: text.unwrap_or(0) as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodeLines {
    pub lines: Vec<CodeLineData>,
}

impl CodeLines {
    pub fn new(stats: CodeStats) -> Self {
        CodeLines {
            lines: stats
                .code_lines
                .iter()
                .map(|line| CodeLineData::new(line))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokei::{Config, LanguageType};

    #[test]
    pub fn can_process_tabs_and_spaces() {
        let data = CodeLineData::new(" \t \t foo".as_bytes());
        assert_eq!(
            data,
            CodeLineData {
                spaces: 3,
                tabs: 2,
                text: 3
            }
        );
    }

    #[test]
    pub fn can_process_unicode() {
        let data = CodeLineData::new("①②③④⑤⑥⑦⑧⑨⑩".as_bytes());
        assert_eq!(
            data,
            CodeLineData {
                spaces: 0,
                tabs: 0,
                text: 10
            }
        );
    }

    #[test]
    pub fn can_parse_source_code() {
        let code = r#"function foo☃() {

    blah;

    // comment
}
/* longer comment
with blanks

yow
*/
foo();"#;
        let stats: CodeStats = LanguageType::JavaScript.parse_from_str(code, &Config::default());

        eprintln!("Stats: {:?}", stats);

        let mut result: CodeLines = CodeLines::new(stats);

        let expected = vec![
            CodeLineData {
                spaces: 0,
                tabs: 0,
                text: 17,
            },
            CodeLineData {
                spaces: 4,
                tabs: 0,
                text: 5,
            },
            CodeLineData {
                spaces: 0,
                tabs: 0,
                text: 1,
            },
            CodeLineData {
                spaces: 0,
                tabs: 0,
                text: 6,
            },
        ]
        .sort_by(|a, b| a.text.partial_cmp(&b.text).unwrap());

        let actual = result
            .lines
            .sort_by(|a, b| a.text.partial_cmp(&b.text).unwrap());
        assert_eq!(actual, expected);
    }
}
