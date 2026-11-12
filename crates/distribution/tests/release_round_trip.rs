//! Updater and channel acceptance for the tags one bump-to-stable round trip
//! produces. `scripts/release-round-trip-e2e.sh` cuts the tags with the real
//! release scripts in a temporary repository and hands them to the ignored
//! test through the environment; the fixed-tag test keeps the same policy
//! under `cargo test --workspace` and ROADMAP.toml evidence.
use plexi_distribution::release::{Asset, Release, accepts, archive_name, select};

const PLATFORM: &str = "macos-arm64";

struct Tags {
    alpha: String,
    beta: String,
    stable: String,
}

/// A published release carries every channel archive plus checksum, the way
/// `scripts/publish-release.sh` attaches them; `prerelease` mirrors its
/// `*-*` rule.
fn published(tag: &str) -> Release {
    let assets = ["stable", "beta", "alpha"]
        .iter()
        .flat_map(|channel| {
            let name = archive_name(PLATFORM, channel);
            [
                Asset {
                    name: format!("{name}.sha256"),
                },
                Asset { name },
            ]
        })
        .collect();
    Release {
        tag_name: tag.into(),
        draft: false,
        prerelease: tag.contains('-'),
        assets,
    }
}

fn picked(releases: &[Release], channel: &str, current: Option<&str>) -> Option<String> {
    select(releases, channel, PLATFORM, current).map(|r| r.tag_name.clone())
}

fn assert_policy(tags: &Tags) {
    let Tags {
        alpha,
        beta,
        stable,
    } = tags;

    assert!(
        accepts("alpha", alpha, true) && accepts("alpha", beta, true),
        "alpha channel must accept alpha and beta prereleases"
    );
    assert!(
        accepts("beta", beta, true) && !accepts("beta", alpha, true),
        "beta channel must accept beta and reject alpha prereleases"
    );
    assert!(
        accepts("stable", stable, false)
            && !accepts("stable", beta, true)
            && !accepts("stable", alpha, true),
        "stable channel must accept only the stable tag"
    );

    // Alpha and beta are published, stable is not yet: stable has nothing to
    // install, beta takes beta, alpha prefers its own prerelease.
    let before_stable = vec![published(alpha), published(beta)];
    assert_eq!(picked(&before_stable, "stable", None), None);
    assert_eq!(
        picked(&before_stable, "beta", None).as_deref(),
        Some(beta.as_str())
    );
    assert_eq!(
        picked(&before_stable, "alpha", None).as_deref(),
        Some(alpha.as_str())
    );

    // Stable is published: stable and beta converge on it, while alpha keeps
    // its prerelease on a fresh install and never demotes to the same-series stable.
    let after_stable = vec![published(alpha), published(beta), published(stable)];
    assert_eq!(
        picked(&after_stable, "stable", None).as_deref(),
        Some(stable.as_str())
    );
    assert_eq!(
        picked(&after_stable, "beta", Some(beta)).as_deref(),
        Some(stable.as_str())
    );
    assert_eq!(
        picked(&after_stable, "alpha", None).as_deref(),
        Some(alpha.as_str())
    );
    assert_eq!(
        picked(&after_stable, "alpha", Some(alpha)).as_deref(),
        Some(beta.as_str()),
        "an installed alpha moves up to the beta prerelease, never to stable"
    );
    assert_eq!(picked(&after_stable, "alpha", Some(beta)), None);

    // A draft (a release whose asset gates have not all passed) is invisible.
    let mut draft = published(stable);
    draft.draft = true;
    assert_eq!(picked(&[draft], "stable", None), None);
}

#[test]
fn release_round_trip_tag_lane_policy() {
    assert_policy(&Tags {
        alpha: "v1.2.3-alpha.1".into(),
        beta: "v1.2.3-beta.1".into(),
        stable: "v1.2.3".into(),
    });
}

#[test]
#[ignore = "driven by scripts/release-round-trip-e2e.sh, which supplies the tags"]
fn release_round_trip_script_tags_are_accepted() {
    let var = |name: &str| {
        std::env::var(name).unwrap_or_else(|e| {
            panic!("{name} must be set by scripts/release-round-trip-e2e.sh: {e}")
        })
    };
    assert_policy(&Tags {
        alpha: var("PLEXI_ROUND_TRIP_ALPHA_TAG"),
        beta: var("PLEXI_ROUND_TRIP_BETA_TAG"),
        stable: var("PLEXI_ROUND_TRIP_STABLE_TAG"),
    });
}
