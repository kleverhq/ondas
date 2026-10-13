use std::{fs::{self, File}, hint::black_box, io::Write, ops::ControlFlow, path::PathBuf};
use criterion::{Criterion, SamplingMode};
use ondas::{Sample, ScanRef, Selection, Signal, Time, TimeRange, ValueRef, Waveform};
use sha2::{Digest, Sha256};

#[derive(Clone)]
struct Pick { path: String, slice: Option<(u32,u32)> }
fn pick(path: &str) -> Pick { Pick { path:path.into(),slice:None } }
fn probes(n: usize) -> Vec<Pick> { (0..n).map(|i| pick(&format!("top.probes({i}).probe"))).collect() }
fn range(a:u64,b:u64)->TimeRange { TimeRange::closed(Time::from_ticks(a),Time::from_ticks(b)) }
#[derive(Clone)]
enum Op { Scan(u64,u64), First(u64,u64), Candidates(u64,u64), Trace(u64,u64), Point(u64), Series([u64;3]) }
struct Case { name:String, picks:Vec<Pick>, op:Op }

fn paths(fixture:&str)->(PathBuf,PathBuf) {
    let ghw = if fixture=="dense4096" { PathBuf::from("tmp/ghw-comparison/dense4096/waveform.ghw") }
        else { PathBuf::from(format!("fixtures/ghw/{fixture}/waveform.ghw")) };
    (ghw,PathBuf::from(format!("tmp/ghw-comparison/{fixture}/waveform.fst")))
}

fn selected(source:&Waveform,fst:&Waveform,picks:&[Pick])->(Vec<Signal>,Vec<Signal>) {
    let bases=source.hierarchy().signals().collect::<Vec<_>>();
    let mut a=Vec::new(); let mut b=Vec::new();
    for p in picks {
        let original=source.hierarchy().signal(&p.path).unwrap();
        let i=bases.iter().position(|s| *s==original).unwrap();
        let converted=fst.hierarchy().signal(&format!("export.s{i}")).unwrap();
        assert_eq!(original.encoding(),converted.encoding());
        match p.slice {
            Some((msb,lsb))=>{a.push(original.slice(msb,lsb).unwrap()); b.push(converted.slice(msb,lsb).unwrap());}
            None=>{a.push(original);b.push(converted);}
        }
    }
    (a,b)
}

fn value(hash:&mut Sha256,v:ValueRef<'_>) {
    match v {
        ValueRef::Bits(bits)=>{hash.update([0]);hash.update(bits.width().to_le_bytes());hash.update(bits.to_string().as_bytes());}
        ValueRef::Real(real)=>{hash.update([1]);hash.update(real.to_bits().to_le_bytes());}
        _=>panic!("unexpected value"),
    }
}
fn preflight(selection:&mut Selection<'_>,op:&Op)->(u64,String) {
    let mut hash=Sha256::new();let mut rows=0;
    match op {
        Op::Point(t)|Op::Series([t,_,_])=>{
            let ticks=match op {Op::Series(ticks)=>ticks.to_vec(),_=>vec![*t]};
            for t in ticks {
                for sample in selection.samples(Time::from_ticks(t)).unwrap() {
                    let Sample::Value {value:v,changed_at,..}=sample else {panic!("missing sample")};
                    hash.update(t.to_le_bytes());hash.update(changed_at.map(|t|t.ticks()).unwrap_or(u64::MAX).to_le_bytes());value(&mut hash,v.as_ref());rows+=1;
                }
            }
        }
        Op::Candidates(a,b)=>{
            let _=selection.scan_candidate_times(range(*a,*b),|t| {rows+=1;hash.update(t.ticks().to_le_bytes());ControlFlow::<()>::Continue(())}).unwrap();
        }
        Op::Scan(a,b)|Op::First(a,b)|Op::Trace(a,b)=>{
            let _=selection.scan_each(range(*a,*b),|slot,record| {
                hash.update((slot as u64).to_le_bytes());
                match record {
                    ScanRef::Initial {value:v,changed_at,..}=>{hash.update([0]);hash.update(changed_at.map(|t|t.ticks()).unwrap_or(u64::MAX).to_le_bytes());value(&mut hash,v);}
                    ScanRef::Change {time,value:v,..}=>{hash.update([1]);hash.update(time.ticks().to_le_bytes());value(&mut hash,v);}
                    _=>unreachable!(),
                }
                rows+=1;
                if matches!(op,Op::First(..)) {ControlFlow::Break(())} else {ControlFlow::Continue(())}
            }).unwrap();
        }
    }
    (rows,format!("{:x}",hash.finalize()))
}

fn run(selection:&mut Selection<'_>,op:&Op)->u64 {
    match op {
        Op::Scan(a,b)=>{let mut n=0;let _=selection.scan_each(range(*a,*b),|slot,record| {black_box((slot,record));n+=1;ControlFlow::<()>::Continue(())}).unwrap();n}
        Op::First(a,b)=>{let flow=selection.scan_each(range(*a,*b),|slot,record| {black_box((slot,record));ControlFlow::Break(())}).unwrap();u64::from(flow.is_break())}
        Op::Candidates(a,b)=>{let mut n=0;let _=selection.scan_candidate_times(range(*a,*b),|t|{black_box(t);n+=1;ControlFlow::<()>::Continue(())}).unwrap();n}
        Op::Trace(a,b)=>{drop(black_box(selection.traces(range(*a,*b)).unwrap()));0}
        Op::Point(t)=>{drop(black_box(selection.samples(Time::from_ticks(*t)).unwrap()));1}
        Op::Series(ticks)=>{for t in ticks {drop(black_box(selection.samples(Time::from_ticks(*t)).unwrap()));}3}
    }
}

fn add(cases:&mut Vec<Case>,name:&str,picks:Vec<Pick>,op:Op){cases.push(Case{name:name.into(),picks,op});}
fn cases(fixture:&str,end:u64)->Vec<Case> {
    let mut out=Vec::new();
    if fixture=="ghw0097-wide-toggle" {
        let whole=vec![pick("top.wide")];let scalar=vec![pick("top.control")];
        for (position,a,b) in [("early",1,129),("late",3968,4096)] {
            add(&mut out,&format!("whole/scan/{position}"),whole.clone(),Op::Scan(a,b));
            add(&mut out,&format!("scalar/scan/{position}"),scalar.clone(),Op::Scan(a,b));
        }
        for (name,msb,lsb) in [("low",0,0),("stable-high",4095,1)] {add(&mut out,&format!("{name}/scan/late"),vec![Pick{path:"top.wide".into(),slice:Some((msb,lsb))}],Op::Scan(3968,4096));}
        add(&mut out,"whole/point/end",whole.clone(),Op::Point(end));
        add(&mut out,"whole/series/repeat-end",whole.clone(),Op::Series([end-2;3]));
        add(&mut out,"whole/candidates/late",whole.clone(),Op::Candidates(3968,4096));
        add(&mut out,"whole/trace/late",whole.clone(),Op::Trace(3968,4096));
        add(&mut out,"whole/first-record",whole,Op::First(1,4096));
    } else if fixture.starts_with("ghw0098")||fixture.starts_with("ghw0099")||fixture.starts_with("ghw0100")||fixture.starts_with("ghw0101")||fixture=="dense4096" {
        let late=end-64;
        add(&mut out,"probes1/scan/early",probes(1),Op::Scan(1,65));
        for n in [1,16,64] {add(&mut out,&format!("probes{n}/scan/late"),probes(n),Op::Scan(late,end));}
        add(&mut out,"noise/scan/full",vec![pick("top.noise")],Op::Scan(0,end));
        add(&mut out,"constant/scan/late",vec![pick("top.constant_probe")],Op::Scan(late,end));
        add(&mut out,"probes1/point/end",probes(1),Op::Point(end));
        add(&mut out,"probes1/candidates/late",probes(1),Op::Candidates(late,end));
        add(&mut out,"probes1/first-record/late",probes(1),Op::First(late,end));
    } else {
        let names=match fixture {
            "ghw0001-wellen-ghw-bug"=>["test_rom_tb.clk","test_rom_tb.bus_ctrl.addr"],
            "ghw0090-ws2815b-driver"=>["ws2815b_driver_tb.clock_50","ws2815b_driver_tb.serial_out"],
            "ghw0094-surfer-issue-547"=>["cpu_core_tb.uut.reg_file.addr_c","cpu_core_tb.uut.reg_file.addr_c"],
            _=>unreachable!(),
        };
        let one=vec![pick(names[0])];let two=if names[0]==names[1] {one.clone()} else {vec![pick(names[0]),pick(names[1])]};
        let width=end/32;
        add(&mut out,"selected1/scan/early",one.clone(),Op::Scan(0,width));
        add(&mut out,"selected1/scan/late",one.clone(),Op::Scan(end-width,end));
        add(&mut out,"selected1/scan/full",one.clone(),Op::Scan(0,end));
        add(&mut out,"selected/scan/full",two.clone(),Op::Scan(0,end));
        add(&mut out,"selected1/point/end",one.clone(),Op::Point(end));
        add(&mut out,"selected1/candidates/late",one.clone(),Op::Candidates(end-width,end));
        add(&mut out,"selected1/first-record/late",one.clone(),Op::First(end-width,end));
        add(&mut out,"selected1/series/backstep-end",one,Op::Series([end-1,end,end-1]));
        if fixture=="ghw0094-surfer-issue-547" {add(&mut out,"addr/scan/verified-active",vec![pick(names[0])],Op::Scan(149_000_000,153_000_000));}
    }
    out
}

fn main() {
    let mut c=Criterion::default().configure_from_args();
    let fst_first=std::env::var_os("ONDAS_COMPARISON_FST_FIRST").is_some();
    let confirm=std::env::var_os("ONDAS_COMPARISON_CONFIRM").is_some();
    let suffix=std::env::var("ONDAS_COMPARISON_ROUND").unwrap();
    let mut evidence=File::create(format!("tmp/ghw-comparison/preflight-{suffix}.tsv")).unwrap();
    let fixtures=["ghw0097-wide-toggle","ghw0098-topology-short","ghw0099-topology-long","ghw0100-topology-many","ghw0101-topology-dense","dense4096","ghw0001-wellen-ghw-bug","ghw0090-ws2815b-driver","ghw0094-surfer-issue-547"];
    for fixture in fixtures {
        let (ghw_path,fst_path)=paths(fixture);
        if fixture!="dense4096" {
            let sidecar:serde_json::Value=serde_json::from_slice(&fs::read(format!("fixtures/ghw/{fixture}/fixture.json")).unwrap()).unwrap();
            let bytes=fs::read(&ghw_path).unwrap();assert_eq!(bytes.len() as u64,sidecar["artifact"]["size"].as_u64().unwrap());
            assert_eq!(format!("{:x}",Sha256::digest(bytes)),sidecar["artifact"]["sha256"].as_str().unwrap());
        }
        let mut source=ondas::open_with(&ghw_path,"ghw-native").unwrap();let mut fst=ondas::open_with(&fst_path,"fst-lib").unwrap();
        assert_eq!(source.metadata().time_span(),fst.metadata().time_span());
        let end=source.metadata().time_span().unwrap().last().ticks();
        let mut group=c.benchmark_group(format!("ghw-comparison/{fixture}"));
        group.sample_size(if fixture=="ghw0090-ws2815b-driver" {if confirm {20} else {10}} else {if confirm {40} else {20}});
        group.sampling_mode(SamplingMode::Flat);
        group.warm_up_time(std::time::Duration::from_millis(if confirm {500} else {200}));group.measurement_time(std::time::Duration::from_millis(if confirm {1500} else {800}));
        for backend in if fst_first { ["fst-lib","ghw-native"] } else { ["ghw-native","fst-lib"] } {
            let path=if backend=="fst-lib" {&fst_path} else {&ghw_path};
            group.bench_function(format!("open-drop/{backend}"),|b|b.iter(||drop(black_box(ondas::open_with(path,backend).unwrap()))));
        }
        for case in cases(fixture,end) {
            let (a,b)=selected(&source,&fst,&case.picks);let mut sa=source.select(&a).unwrap();let mut sb=fst.select(&b).unwrap();
            let pa=preflight(&mut sa,&case.op);let pb=preflight(&mut sb,&case.op);
            if !matches!(case.op,Op::Candidates(..)) {assert_eq!(pa,pb,"{} {}",fixture,case.name);}
            writeln!(evidence,"{fixture}\t{}\t{}\t{}\t{}\t{}",case.name,pa.0,pb.0,pa.1,pb.1).unwrap();evidence.flush().unwrap();
            for backend in if fst_first { ["fst-lib","ghw-native"] } else { ["ghw-native","fst-lib"] } {
                let selection=if backend=="fst-lib" {&mut sb} else {&mut sa};
                group.bench_function(format!("{}/{backend}",case.name),|b| b.iter(||black_box(run(selection,&case.op))));
            }
        }
        group.finish();
    }
    c.final_summary();
}
