//! The links Quadrant opens, after the desktop app's `deepLinks.ts`: the
//! `curseforge:`, `modrinth:` and `quadrantnext:` schemes the installers
//! register, and Quadrant Share's `https://usequadrant.dev/modpack/<code>`.
//! Pure parsing; `app.rs` carries the actions out.

use quadrant_core::models::ModSource;
use url::Url;

use crate::pages::parse_share_code;

/// What a link asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open a mod's install page.
    InstallMod(ModLink),
    /// Quadrant ID's redirect; the caller still checks `state` against the
    /// sign-in it started.
    OauthLogin {
        state: Option<String>,
        code: Option<String>,
        redirect_uri: String,
    },
    /// Open the Quadrant Share import of this code.
    ImportModpack(String),
    /// A Quadrant scheme with nothing usable in it: say so and stop.
    Unsupported,
    /// Not a Quadrant link: skip it and go on.
    Ignore,
}

/// A mod a link points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModLink {
    pub source: ModSource,
    pub id: String,
    /// A specific file to install instead of picking a version.
    pub file_id: Option<String>,
    /// Whether the links after this one are dropped. The bare `curseforge:`
    /// scheme always went on to the next link; the others stop.
    pub stop_after: bool,
}

/// What `deepLinks.ts`'s `resolveDeepLink` makes of `raw`; an error where it
/// throws, for a string that isn't a URL.
pub fn resolve(raw: &str) -> Result<Action, url::ParseError> {
    let url = Url::parse(raw)?;
    Ok(match url.scheme() {
        "curseforge" => {
            let action = host(&url)
                .or_else(|| path_parts(&url).into_iter().next())
                .unwrap_or_default()
                .to_lowercase();
            let id = query(&url, "addonId").unwrap_or_default();
            let id = id.trim();
            if action != "install" || id.is_empty() {
                return Ok(Action::Unsupported);
            }
            Action::InstallMod(ModLink {
                source: ModSource::CurseForge,
                id: id.to_string(),
                file_id: query(&url, "fileId"),
                stop_after: false,
            })
        }
        "modrinth" => {
            let mut parts: Vec<String> = host(&url).into_iter().chain(path_parts(&url)).collect();
            // A wrapped website link, such as modrinth://https://modrinth.com/mod/sodium.
            if parts
                .first()
                .is_some_and(|part| part.eq_ignore_ascii_case("https"))
            {
                parts.remove(0);
            }
            if parts
                .first()
                .is_some_and(|part| part.eq_ignore_ascii_case("modrinth.com"))
            {
                parts.remove(0);
            }
            let action = parts.first().map(|part| part.to_lowercase());
            match (action.as_deref(), parts.get(1)) {
                (Some("mod" | "resourcepack" | "shader"), Some(id)) => {
                    Action::InstallMod(ModLink {
                        source: ModSource::Modrinth,
                        id: id.clone(),
                        file_id: None,
                        stop_after: true,
                    })
                }
                _ => Action::Unsupported,
            }
        }
        "quadrantnext" => {
            let action = quadrant_action(&url);
            if url.path().split('/').any(|part| part == "login") || action == "login" {
                return Ok(Action::OauthLogin {
                    state: query(&url, "state"),
                    code: query(&url, "code"),
                    redirect_uri: raw
                        .split('#')
                        .next()
                        .and_then(|raw| raw.split('?').next())
                        .unwrap_or_default()
                        .to_string(),
                });
            }
            let source = match action.as_str() {
                "modrinth" => Some(ModSource::Modrinth),
                "curseforge" => Some(ModSource::CurseForge),
                _ => None,
            };
            if let Some(source) = source {
                let id = mod_id(&url);
                if id.is_empty() {
                    return Ok(Action::Unsupported);
                }
                return Ok(Action::InstallMod(ModLink {
                    source,
                    id,
                    file_id: query(&url, "fileId"),
                    stop_after: true,
                }));
            }
            if action == "modpack" {
                let code = share_code(&url);
                if code.is_empty() {
                    return Ok(Action::Unsupported);
                }
                return Ok(Action::ImportModpack(code));
            }
            Action::Ignore
        }
        "https" => parse_share_code(raw).map_or(Action::Ignore, Action::ImportModpack),
        _ => Action::Ignore,
    })
}

/// The actions a batch of links comes to, in order, as `App.tsx`'s
/// `onOpenUrl` loop runs them: links that aren't Quadrant's are skipped, an
/// unusable or unreadable one ends the batch with [`Action::Unsupported`],
/// and so does anything but a `curseforge:` install.
pub fn plan(urls: &[String]) -> Vec<Action> {
    let mut actions = Vec::new();
    for url in urls {
        let action = resolve(url).unwrap_or(Action::Unsupported);
        let more = match &action {
            Action::Ignore => continue,
            Action::InstallMod(link) => !link.stop_after,
            Action::OauthLogin { .. } | Action::ImportModpack(_) | Action::Unsupported => false,
        };
        actions.push(action);
        if !more {
            break;
        }
    }
    actions
}

/// The URL's host, or `None` where `URL.host` would be empty.
fn host(url: &Url) -> Option<String> {
    url.host_str()
        .filter(|host| !host.is_empty())
        .map(str::to_string)
}

fn path_parts(url: &Url) -> Vec<String> {
    url.path()
        .split('/')
        .filter(|part| !part.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// The first value of `key` in the query, decoded, as `searchParams.get`.
fn query(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

/// `getQuadrantAction`: the host, else the first path segment.
fn quadrant_action(url: &Url) -> String {
    host(url)
        .or_else(|| path_parts(url).into_iter().next())
        .unwrap_or_default()
        .to_lowercase()
}

/// `getQuadrantPathValue`: the segment after the action.
fn path_value(url: &Url) -> Option<String> {
    let parts = path_parts(url);
    let index = if host(url).is_some() { 0 } else { 1 };
    parts.into_iter().nth(index)
}

/// `getQuadrantModId`: `modId`, then `addonId`, then the path.
fn mod_id(url: &Url) -> String {
    query(url, "modId")
        .or_else(|| query(url, "addonId"))
        .or_else(|| path_value(url))
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// `getQuadrantCode`: `code`, then `sharedCode`, then the path.
fn share_code(url: &Url) -> String {
    query(url, "code")
        .or_else(|| query(url, "sharedCode"))
        .or_else(|| path_value(url))
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(source: ModSource, id: &str, file_id: Option<&str>, stop_after: bool) -> Action {
        Action::InstallMod(ModLink {
            source,
            id: id.into(),
            file_id: file_id.map(str::to_string),
            stop_after,
        })
    }

    fn resolved(raw: &str) -> Action {
        resolve(raw).unwrap()
    }

    #[test]
    fn a_curseforge_install_keeps_iterating() {
        assert_eq!(
            resolved("curseforge://install?addonId=238222&fileId=99"),
            link(ModSource::CurseForge, "238222", Some("99"), false)
        );
    }

    #[test]
    fn other_curseforge_actions_and_missing_ids_are_unsupported() {
        for raw in [
            "curseforge://open?addonId=1",
            "curseforge://install",
            "curseforge://install?addonId=%20",
        ] {
            assert_eq!(resolved(raw), Action::Unsupported, "{raw}");
        }
    }

    #[test]
    fn modrinth_links_name_the_project() {
        assert_eq!(
            resolved("modrinth://mod/sodium"),
            link(ModSource::Modrinth, "sodium", None, true)
        );
        for raw in [
            "modrinth://resourcepack/faithful",
            "modrinth://shader/complementary",
        ] {
            assert!(matches!(resolved(raw), Action::InstallMod(_)), "{raw}");
        }
        for raw in [
            "modrinth://mod/sodium/",
            "modrinth:///mod/sodium",
            "modrinth://modrinth.com/mod/sodium",
            "modrinth://https://modrinth.com/mod/sodium",
        ] {
            assert_eq!(
                resolved(raw),
                link(ModSource::Modrinth, "sodium", None, true),
                "{raw}"
            );
        }
    }

    #[test]
    fn modrinth_links_without_a_project_are_unsupported() {
        for raw in [
            "modrinth://user/jellysquid",
            "modrinth://mod",
            "modrinth://mod/",
            "modrinth:///mod/",
            "modrinth://user/mod-fan",
            "modrinth://unsupported-mod/sodium",
        ] {
            assert_eq!(resolved(raw), Action::Unsupported, "{raw}");
        }
    }

    #[test]
    fn quadrantnext_login_is_the_oauth_redirect() {
        assert_eq!(
            resolved("quadrantnext://login?state=abc&code=xyz#fragment"),
            Action::OauthLogin {
                state: Some("abc".into()),
                code: Some("xyz".into()),
                redirect_uri: "quadrantnext://login".into(),
            }
        );
    }

    #[test]
    fn quadrantnext_mod_and_modpack_actions() {
        assert_eq!(
            resolved("quadrantnext://modrinth?modId=sodium"),
            link(ModSource::Modrinth, "sodium", None, true)
        );
        assert_eq!(
            resolved("quadrantnext://curseforge?addonId=1&fileId=2"),
            link(ModSource::CurseForge, "1", Some("2"), true)
        );
        assert_eq!(resolved("quadrantnext://modrinth"), Action::Unsupported);
        assert_eq!(
            resolved("quadrantnext://modpack?code=1234567"),
            Action::ImportModpack("1234567".into())
        );
        assert_eq!(resolved("quadrantnext://mystery"), Action::Ignore);
    }

    #[test]
    fn quadrantnext_ids_come_from_the_query_then_the_path() {
        let id = |raw: &str| mod_id(&Url::parse(raw).unwrap());
        let code = |raw: &str| share_code(&Url::parse(raw).unwrap());
        assert_eq!(id("quadrantnext://modrinth?modId=a"), "a");
        assert_eq!(id("quadrantnext://curseforge?addonId=b"), "b");
        assert_eq!(id("quadrantnext://modrinth/sodium"), "sodium");
        assert_eq!(id("quadrantnext:///modrinth/sodium"), "sodium");
        assert_eq!(code("quadrantnext://modpack?code=111"), "111");
        assert_eq!(code("quadrantnext://modpack?sharedCode=222"), "222");
    }

    #[test]
    fn share_links_import_seven_digit_codes_from_quadrant_only() {
        assert_eq!(
            resolved("https://usequadrant.dev/modpack/1234567"),
            Action::ImportModpack("1234567".into())
        );
        assert_eq!(
            resolved("https://www.usequadrant.dev/modpack/7654321"),
            Action::ImportModpack("7654321".into())
        );
        assert_eq!(
            resolved("https://usequadrant.dev/modpack/12"),
            Action::Ignore
        );
        assert_eq!(
            resolved("https://evil.example.com/modpack/1234567"),
            Action::Ignore
        );
    }

    #[test]
    fn the_packages_register_every_scheme() {
        let schemes = ["quadrantnext", "curseforge", "modrinth"];
        let quoted = schemes.map(|scheme| format!("\"{scheme}\""));
        let manifest = include_str!("../Cargo.toml");
        assert!(
            manifest.contains(&format!("schemes = [{}]", quoted.join(", "))),
            "Cargo.toml's deep-link-protocols"
        );
        let desktop = include_str!("../packaging/linux/quadrant-native.desktop");
        let handlers = schemes.map(|scheme| format!("x-scheme-handler/{scheme};"));
        assert!(
            desktop.contains(&format!("MimeType={}", handlers.concat())),
            "{desktop}"
        );
        assert!(desktop.contains("Exec=quadrant-native %U"), "{desktop}");
    }

    #[test]
    fn non_urls_fail_and_other_schemes_are_ignored() {
        assert!(resolve("not a url").is_err());
        assert_eq!(resolved("mailto:someone@example.com"), Action::Ignore);
    }

    #[test]
    fn a_batch_skips_foreign_links_and_stops_where_the_desktop_app_does() {
        let urls = |list: &[&str]| list.iter().map(|url| url.to_string()).collect::<Vec<_>>();
        assert_eq!(
            plan(&urls(&[
                "mailto:a@b.c",
                "curseforge://install?addonId=1",
                "curseforge://install?addonId=2",
                "modrinth://mod/sodium",
                "modrinth://mod/lithium",
            ])),
            [
                link(ModSource::CurseForge, "1", None, false),
                link(ModSource::CurseForge, "2", None, false),
                link(ModSource::Modrinth, "sodium", None, true),
            ]
        );
        assert_eq!(
            plan(&urls(&["not a url", "modrinth://mod/sodium"])),
            [Action::Unsupported]
        );
        assert_eq!(
            plan(&urls(&[
                "https://usequadrant.dev/modpack/1234567",
                "modrinth://mod/a"
            ])),
            [Action::ImportModpack("1234567".into())]
        );
        assert!(plan(&urls(&["C:\\Users\\someone\\file.txt"])).is_empty());
    }
}
