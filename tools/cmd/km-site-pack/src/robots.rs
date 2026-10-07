//! What a site's `robots.txt` asks of this program.

/// One `Allow` or `Disallow` line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    allow: bool,
    pattern: String,
}

/// The rules of a `robots.txt` that apply to one program, and the sitemaps the file names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Robots {
    rules: Vec<Rule>,
    /// The `Sitemap:` lines, as written.
    pub sitemaps: Vec<String>,
    /// The wait the site asks a program to leave between two requests, when it asks.
    pub crawl_delay: Option<std::time::Duration>,
}

impl Robots {
    /// Reads a `robots.txt` for the program named `agent`.
    ///
    /// A group naming the program is the one that applies. The `*` group applies when no group
    /// names it.
    #[must_use]
    pub fn parse(text: &str, agent: &str) -> Self {
        let agent = agent.to_ascii_lowercase();
        let mut own: Vec<Rule> = Vec::new();
        let mut any: Vec<Rule> = Vec::new();
        let mut named = false;
        let mut sitemaps = Vec::new();
        let (mut own_delay, mut any_delay) = (None, None);

        // Whether the group being read names this program, names `*`, and whether its agent lines
        // are still running. A rule line ends the run, so the next agent line opens a new group.
        let (mut for_own, mut for_any, mut in_agents) = (false, false, false);

        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((field, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match field.trim().to_ascii_lowercase().as_str() {
                "user-agent" => {
                    if !in_agents {
                        (for_own, for_any) = (false, false);
                    }
                    in_agents = true;
                    let token = value.to_ascii_lowercase();
                    if token == "*" {
                        for_any = true;
                    } else if !token.is_empty() && agent.contains(&token) {
                        for_own = true;
                        named = true;
                    }
                }
                field @ ("allow" | "disallow") => {
                    in_agents = false;
                    // An empty `Disallow:` forbids nothing, which is what no rule says as well.
                    if value.is_empty() {
                        continue;
                    }
                    let rule = Rule {
                        allow: field == "allow",
                        pattern: value.to_owned(),
                    };
                    if for_own {
                        own.push(rule.clone());
                    }
                    if for_any {
                        any.push(rule);
                    }
                }
                "sitemap" => sitemaps.push(value.to_owned()),
                "crawl-delay" => {
                    in_agents = false;
                    let seconds = value
                        .parse::<f64>()
                        .ok()
                        .filter(|s| *s > 0.0)
                        .and_then(|s| std::time::Duration::try_from_secs_f64(s).ok());
                    if for_own {
                        own_delay = seconds.or(own_delay);
                    }
                    if for_any {
                        any_delay = seconds.or(any_delay);
                    }
                }
                _ => in_agents = false,
            }
        }

        Self {
            rules: if named { own } else { any },
            sitemaps,
            crawl_delay: if named { own_delay } else { any_delay },
        }
    }

    /// Whether this program may ask for `path`, which is a URL's path with its query.
    ///
    /// The longest rule that matches decides, and `Allow` wins a tie.
    #[must_use]
    pub fn allows(&self, path: &str) -> bool {
        let mut best: Option<&Rule> = None;
        for rule in &self.rules {
            if !matches(&rule.pattern, path) {
                continue;
            }
            let better = best.is_none_or(|held| {
                rule.pattern.len() > held.pattern.len()
                    || (rule.pattern.len() == held.pattern.len() && rule.allow)
            });
            if better {
                best = Some(rule);
            }
        }
        best.is_none_or(|rule| rule.allow)
    }
}

/// Whether a rule's pattern matches a path: a prefix, with `*` for any run and `$` for the end.
fn matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(rest) => (rest, true),
        None => (pattern, false),
    };
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = path.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    for (index, part) in parts.iter().enumerate() {
        let last = index + 1 == parts.len();
        if last && anchored {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    !anchored || rest.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "km-site-pack/1.0";

    #[test]
    fn the_star_group_applies_when_no_group_names_the_program() {
        let robots = Robots::parse(
            "User-agent: OtherBot\nDisallow: /\n\nUser-agent: *\nDisallow: /private/\n",
            AGENT,
        );
        assert!(robots.allows("/songs/a.kar"));
        assert!(!robots.allows("/private/a.kar"));
    }

    #[test]
    fn a_group_naming_the_program_replaces_the_star_group() {
        let robots = Robots::parse(
            "User-agent: *\nDisallow: /\n\nUser-agent: km-site-pack\nDisallow: /b/\n",
            AGENT,
        );
        assert!(robots.allows("/a/x.kar"));
        assert!(!robots.allows("/b/x.kar"));
    }

    #[test]
    fn several_agent_lines_share_one_group() {
        let robots = Robots::parse(
            "User-agent: OtherBot\nUser-agent: *\nDisallow: /x/\n",
            AGENT,
        );
        assert!(!robots.allows("/x/a"));
    }

    #[test]
    fn the_longest_rule_decides_and_allow_wins_a_tie() {
        let robots = Robots::parse(
            "User-agent: *\nDisallow: /a/\nAllow: /a/open/\nDisallow: /t\nAllow: /t\n",
            AGENT,
        );
        assert!(!robots.allows("/a/closed.kar"));
        assert!(robots.allows("/a/open/x.kar"));
        assert!(robots.allows("/t"));
    }

    #[test]
    fn an_empty_disallow_and_an_empty_file_forbid_nothing() {
        assert!(Robots::parse("User-agent: *\nDisallow:\n", AGENT).allows("/a"));
        assert!(Robots::parse("", AGENT).allows("/a"));
    }

    #[test]
    fn a_star_and_an_end_anchor_are_honoured() {
        let robots = Robots::parse(
            "User-agent: *\nDisallow: /*.php$\nDisallow: /tmp*/x\n",
            AGENT,
        );
        assert!(!robots.allows("/a/get.php"));
        assert!(robots.allows("/a/get.php?x=1"));
        assert!(!robots.allows("/tmp123/x/y"));
        assert!(robots.allows("/tmp123/y"));
    }

    #[test]
    fn the_delay_a_site_asks_for_is_read_from_the_group_that_applies() {
        let robots = Robots::parse(
            "User-agent: OtherBot\nCrawl-delay: 30\n\nUser-agent: *\nCrawl-delay: 2.5\nDisallow: /x\n",
            AGENT,
        );
        assert_eq!(
            robots.crawl_delay,
            Some(std::time::Duration::from_millis(2500))
        );
        assert!(!robots.allows("/x"));
        assert_eq!(
            Robots::parse("User-agent: *\nCrawl-delay: soon\n", AGENT).crawl_delay,
            None
        );
    }

    #[test]
    fn sitemap_lines_and_comments_are_read() {
        let robots = Robots::parse(
            "Sitemap: http://127.0.0.1/sitemap.xml # the map\n# nothing\nUser-agent: *\n",
            AGENT,
        );
        assert_eq!(robots.sitemaps, ["http://127.0.0.1/sitemap.xml"]);
    }
}
