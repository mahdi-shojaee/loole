use serde::{ser::SerializeStruct, Serialize};
use tokio::runtime::Runtime;

use crate::{
    bench_utils::{BenchError, BenchResult},
    benchmark::{
        bench_async_async, bench_async_sync, bench_sync_async, bench_sync_sync, CrateName,
        BUFFER_SIZE_LIST,
    },
    MAX_RECEIVERS, MAX_SENDERS,
};
use num_format::{Locale, ToFormattedString};

#[derive(Serialize, Debug)]
pub struct BenchData {
    mpsc: DataSets,
    mpmc: DataSets,
    spsc: DataSets,
}

#[derive(Debug)]
pub struct DataSets {
    pub sync_sync: ChartData,
    pub async_async: ChartData,
    pub async_sync: ChartData,
    pub sync_async: ChartData,
}

impl Serialize for DataSets {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut obj = serializer.serialize_struct("DataSets", 4)?;
        obj.serialize_field("sync-sync", &self.sync_sync)?;
        obj.serialize_field("async-async", &self.async_async)?;
        obj.serialize_field("async-sync", &self.async_sync)?;
        obj.serialize_field("sync-async", &self.sync_async)?;
        obj.end()
    }
}

#[derive(Serialize, Debug)]
pub struct ChartData {
    title: String,
    dataset: DataSet,
}

#[derive(Serialize, Debug)]
pub struct DataSet {
    source: Vec<Vec<Item>>,
}

#[derive(Debug)]
pub enum Item {
    Label(String),
    Value(usize),
}

impl Serialize for Item {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Item::Label(l) => serializer.serialize_str(l),
            Item::Value(v) => serializer.serialize_u64(*v as u64),
        }
    }
}

const CRATES: [CrateName; 6] = [
    CrateName::Loole,
    CrateName::Flume,
    CrateName::AsyncChannel,
    CrateName::Tokio,
    CrateName::Kanal,
    CrateName::CrossbeamChannel,
];

fn f(n: usize) -> String {
    n.to_formatted_string(&Locale::en)
}

pub fn get_dataset<F>(benchmark: F) -> DataSet
where
    F: Fn(CrateName, Option<usize>) -> Result<BenchResult, BenchError>,
{
    let mut source = vec![];
    eprint!("{:<17}\t", "capacity");
    let mut row = vec![Item::Label("capacity".to_string())];
    for crate_name in CRATES {
        eprint!("{:<17}\t", crate_name.to_string());
        row.push(Item::Label(crate_name.to_string()));
    }
    eprintln!();
    source.push(row);
    for buffer_size in BUFFER_SIZE_LIST {
        eprint!("{:<17}\t", format!("bounded({})", buffer_size.unwrap()));
        let mut row = vec![Item::Label(format!("bounded({})", buffer_size.unwrap()))];
        for crate_name in CRATES {
            let r = match benchmark(crate_name, buffer_size) {
                Ok(bench_result) => bench_result.throughput,
                Err(_) => 0,
            };
            eprint!("{:<17}\t", f(r));
            row.push(Item::Value(r));
        }
        eprintln!();
        source.push(row);
    }
    eprintln!();
    DataSet { source }
}

fn format_title(
    senders_no: usize,
    receivers_no: usize,
    sender_type: &str,
    receiver_type: &str,
) -> String {
    format!(
        "{} {} sender{} / {} {} receiver{}",
        senders_no,
        sender_type,
        if senders_no != 1 { "s" } else { "" },
        receivers_no,
        receiver_type,
        if receivers_no != 1 { "s" } else { "" }
    )
}

pub fn get_datasets(senders_no: usize, receivers_no: usize, rt: &Runtime) -> DataSets {
    let title = format_title(senders_no, receivers_no, "sync", "sync");
    eprintln!("{}", title);
    let sync_sync = ChartData {
        title,
        dataset: get_dataset(|c, b| rt.block_on(bench_sync_sync(c, senders_no, receivers_no, b))),
    };
    let title = format_title(senders_no, receivers_no, "async", "async");
    eprintln!("{}", title);
    let async_async = ChartData {
        title,
        dataset: get_dataset(|c, b| rt.block_on(bench_async_async(c, senders_no, receivers_no, b))),
    };
    let title = format_title(senders_no, receivers_no, "async", "sync");
    eprintln!("{}", title);
    let async_sync = ChartData {
        title,
        dataset: get_dataset(|c, b| rt.block_on(bench_async_sync(c, senders_no, receivers_no, b))),
    };
    let title = format_title(senders_no, receivers_no, "sync", "async");
    eprintln!("{}", title);
    let sync_async = ChartData {
        title,
        dataset: get_dataset(|c, b| rt.block_on(bench_sync_async(c, senders_no, receivers_no, b))),
    };
    DataSets {
        sync_sync,
        async_async,
        async_sync,
        sync_async,
    }
}

pub fn get_bench_data(rt: &Runtime) -> BenchData {
    eprintln!("MPSC\n----");
    let mpsc = get_datasets(MAX_SENDERS, 1, rt);
    eprintln!("MPMC\n----");
    let mpmc = get_datasets(MAX_SENDERS, MAX_RECEIVERS, rt);
    eprintln!("SPSC\n----");
    let spsc = get_datasets(1, 1, rt);
    BenchData { mpsc, mpmc, spsc }
}
