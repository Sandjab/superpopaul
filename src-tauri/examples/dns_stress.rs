//! Repro diagnostic : rejoue naptr_smp_url via Dns::system() sur des
//! hostnames SML connus comme enregistrés, à forte concurrence, et compte
//! les verdicts. Usage :
//!   cargo run --release --example dns_stress -- <fichier_hosts> [concurrence]

use std::sync::Arc;
use superpopaul_lib::direct::{Dns, SmlLookup};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: dns_stress <fichier_hosts> [conc]");
    let conc: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(64);
    let hosts: Vec<String> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect();
    eprintln!("{} hosts, concurrence {conc}", hosts.len());

    // Mode instrumenté : appel hickory direct pour voir les erreurs brutes.
    let raw = std::env::var("RAW").is_ok();
    let resolver = Arc::new(
        hickory_resolver::TokioAsyncResolver::tokio_from_system_conf().unwrap(),
    );
    let dns = Arc::new(Dns::system().unwrap());
    let sem = Arc::new(tokio::sync::Semaphore::new(conc));
    let mut tasks = Vec::new();
    let t0 = std::time::Instant::now();
    for h in hosts {
        let dns = dns.clone();
        let resolver = resolver.clone();
        let sem = sem.clone();
        tasks.push(tokio::spawn(async move {
            let _p = sem.acquire().await.unwrap();
            if raw {
                use hickory_proto::rr::RecordType;
                let r = match resolver.lookup(h.as_str(), RecordType::NAPTR).await {
                    Ok(ans) => SmlLookup::Found(format!("{} answers", ans.iter().count())),
                    Err(e) => SmlLookup::Failed(format!("{:?}", e.kind())),
                };
                return (h, r);
            }
            let r = dns.naptr_smp_url(&h).await;
            (h, r)
        }));
    }
    let (mut found, mut nx, mut failed) = (0u32, 0u32, 0u32);
    let mut nx_hosts = Vec::new();
    let mut fail_msgs: std::collections::BTreeMap<String, u32> = Default::default();
    for t in tasks {
        let (h, r) = t.await.unwrap();
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
    println!(
        "found={found} nxdomain={nx} failed={failed} en {dt:.1}s ({:.0} req/s)",
        (found + nx + failed) as f64 / dt
    );
    for (m, c) in &fail_msgs {
        println!("  FAIL {c}× {m}");
    }
    for h in &nx_hosts {
        println!("  NX {h}");
    }
}
