//! Banc d'essai / diagnostic : rejoue naptr_smp_url sur des hostnames SML
//! connus comme enregistrés, à forte concurrence, et compte les verdicts
//! (un NXDOMAIN sur ces hosts = faux négatif). Usage :
//!   cargo run --release --example dns_stress -- <fichier_hosts> [conc] [résolveur] [rafale_dns]
//! résolveur : « system » (défaut), une IP (DNS classique UDP/TCP sur 53),
//! ou une URL https (DoH RFC 8484). rafale_dns : permis du sémaphore DNS
//! (défaut 32, comme l'app).

use std::sync::Arc;
use superpopaul_lib::direct::{dns_from_spec, SmlLookup, DNS_CONCURRENCY_DEFAULT};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: dns_stress <fichier_hosts> [conc] [résolveur]");
    let conc: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(64);
    let spec = args.next().unwrap_or_else(|| "system".into());
    let dns_conc: usize = args
        .next()
        .map(|s| s.parse().unwrap())
        .unwrap_or(DNS_CONCURRENCY_DEFAULT as usize);
    let hosts: Vec<String> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect();
    eprintln!(
        "{} hosts, concurrence {conc}, résolveur {spec}, rafale DNS {dns_conc}",
        hosts.len()
    );

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    let spec_opt = (spec != "system").then_some(spec.as_str());
    let dns = Arc::new(dns_from_spec(spec_opt, &http).unwrap());
    let dns_sem = Arc::new(tokio::sync::Semaphore::new(dns_conc));
    let sem = Arc::new(tokio::sync::Semaphore::new(conc));
    let mut tasks = Vec::new();
    let t0 = std::time::Instant::now();
    for h in hosts {
        let dns = dns.clone();
        let dns_sem = dns_sem.clone();
        let sem = sem.clone();
        tasks.push(tokio::spawn(async move {
            let _p = sem.acquire().await.unwrap();
            let t = std::time::Instant::now();
            let r = dns.naptr_smp_url(&h, &dns_sem).await;
            (h, r, t.elapsed().as_millis() as u64)
        }));
    }
    let (mut found, mut nx, mut failed) = (0u32, 0u32, 0u32);
    let mut nx_hosts = Vec::new();
    let mut fail_msgs: std::collections::BTreeMap<String, u32> = Default::default();
    let mut lat_ms = Vec::new();
    for t in tasks {
        let (h, r, ms) = t.await.unwrap();
        lat_ms.push(ms);
        match r {
            SmlLookup::Found(_) => found += 1,
            SmlLookup::NotRegistered => {
                nx += 1;
                if nx_hosts.len() < 20 {
                    nx_hosts.push(h);
                }
            }
            SmlLookup::Failed(m) => {
                failed += 1;
                *fail_msgs.entry(m).or_insert(0) += 1;
            }
        }
    }
    let dt = t0.elapsed().as_secs_f64();
    lat_ms.sort_unstable();
    let pct = |p: usize| lat_ms[(lat_ms.len() * p / 100).min(lat_ms.len() - 1)];
    println!(
        "found={found} nxdomain={nx} failed={failed} en {dt:.1}s ({:.0} req/s) — \
         latence ms p50={} p90={} p99={} max={}",
        (found + nx + failed) as f64 / dt,
        pct(50),
        pct(90),
        pct(99),
        lat_ms.last().unwrap()
    );
    for (m, c) in &fail_msgs {
        println!("  FAIL {c}× {m}");
    }
    for h in &nx_hosts {
        println!("  NX {h}");
    }
}
