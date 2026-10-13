use std::{fs::File, io::{BufWriter, Write}, ops::ControlFlow, path::Path};
use ondas::{Encoding, ScanRef, TimeRange, ValueRef};
use sha2::{Digest, Sha256};

fn signals(wave: &ondas::Waveform) -> Vec<ondas::Signal> {
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    assert!(signals.iter().all(|s| matches!(s.encoding(), Encoding::Bits { .. } | Encoding::Real)));
    signals
}

fn histories(wave: &mut ondas::Waveform, selected: &[ondas::Signal]) -> Vec<(u64, String)> {
    let mut hashes = vec![Sha256::new(); selected.len()];
    let mut counts = vec![0u64; selected.len()];
    let flow = wave.select(selected).unwrap().scan_each(TimeRange::all(), |slot, record| {
        let ScanRef::Change { time, value, .. } = record else { panic!("unexpected entering state in full range") };
        counts[slot] += 1;
        hashes[slot].update(time.ticks().to_le_bytes());
        match value {
            ValueRef::Bits(bits) => {
                hashes[slot].update([0]);
                hashes[slot].update(bits.width().to_le_bytes());
                hashes[slot].update(bits.to_string().as_bytes());
            }
            ValueRef::Real(real) => { hashes[slot].update([1]); hashes[slot].update(real.to_bits().to_le_bytes()); }
            _ => unreachable!(),
        }
        ControlFlow::<()>::Continue(())
    }).unwrap();
    assert!(flow.is_continue());
    counts.into_iter().zip(hashes).map(|(n,h)| (n, format!("{:x}",h.finalize()))).collect()
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let mut source = ondas::open_with(&args[2], "ghw-native").unwrap();
    let selected = signals(&source);
    let span = source.metadata().time_span().unwrap();
    let mut first = vec![false; selected.len()];
    match args[1].as_str() {
        "export" => {
            let mut out = BufWriter::new(File::create(&args[3]).unwrap());
            out.write_all(b"OWAV0001").unwrap();
            out.write_all(&(selected.len() as u32).to_le_bytes()).unwrap();
            out.write_all(&span.first().ticks().to_le_bytes()).unwrap();
            out.write_all(&span.last().ticks().to_le_bytes()).unwrap();
            for signal in &selected {
                let (kind, width) = match signal.encoding() { Encoding::Bits { width } => (0,width), Encoding::Real => (1,64), _ => unreachable!() };
                out.write_all(&[kind]).unwrap(); out.write_all(&width.to_le_bytes()).unwrap();
            }
            let mut rows = 0u64;
            let flow = source.select(&selected).unwrap().scan_each(TimeRange::all(), |slot,record| {
                let ScanRef::Change { time,value,.. } = record else { panic!("unexpected initial") };
                if !first[slot] { assert_eq!(time,span.first(),"missing first-tick value"); first[slot] = true; }
                out.write_all(&time.ticks().to_le_bytes()).unwrap();
                out.write_all(&(slot as u32).to_le_bytes()).unwrap();
                match value {
                    ValueRef::Bits(bits) => {
                        out.write_all(&bits.width().to_le_bytes()).unwrap();
                        out.write_all(bits.to_string().as_bytes()).unwrap();
                    }
                    ValueRef::Real(real) => { out.write_all(&8u32.to_le_bytes()).unwrap(); out.write_all(&real.to_bits().to_le_bytes()).unwrap(); }
                    _ => unreachable!(),
                }
                rows += 1;
                ControlFlow::<()>::Continue(())
            }).unwrap();
            assert!(flow.is_continue()); assert!(first.iter().all(|s| *s));
            out.write_all(&span.last().ticks().to_le_bytes()).unwrap();
            out.write_all(&u32::MAX.to_le_bytes()).unwrap(); out.write_all(&0u32.to_le_bytes()).unwrap();
            out.flush().unwrap();
            println!("{} signals={} rows={} first={} last={}",Path::new(&args[2]).display(),selected.len(),rows,span.first().ticks(),span.last().ticks());
        }
        "verify" => {
            let mut fst = ondas::open_with(&args[3],"fst-lib").unwrap();
            assert_eq!(source.metadata().time_span(),fst.metadata().time_span());
            assert_eq!(selected.len(),fst.hierarchy().signals().count());
            let mapped = (0..selected.len()).map(|i| fst.hierarchy().signal(&format!("export.s{i}")).unwrap()).collect::<Vec<_>>();
            for (a,b) in selected.iter().zip(&mapped) { assert_eq!(a.encoding(),b.encoding()); }
            let a = histories(&mut source,&selected);
            let b = histories(&mut fst,&mapped);
            assert_eq!(a,b,"full normalized history mismatch");
            let rows = a.iter().map(|(n,_)| n).sum::<u64>();
            println!("verified signals={} rows={} source={} counterpart={}",selected.len(),rows,args[2],args[3]);
        }
        "verify-dense-vcd" => {
            let mut vcd=ondas::open_with(&args[3],"vcd-native").unwrap();
            let a=source.hierarchy().signal("top.noise").unwrap();
            let b=vcd.hierarchy().signal("top.noise").unwrap();
            assert_eq!(a.width(),Some(4096)); assert_eq!(a.encoding(),b.encoding());
            for tick in [0,1,2,64,65,255,256] {
                let x=source.sample(a,ondas::Time::from_ticks(tick)).unwrap();
                let y=vcd.sample(b,ondas::Time::from_ticks(tick)).unwrap();
                let digit=if tick%2==0 {"0"} else {"1"};
                let expected=digit.repeat(4096);
                for sample in [x,y] {
                    let ondas::Sample::Value{value,..}=sample else {panic!("missing dense value")};
                    let ValueRef::Bits(bits)=value.as_ref() else {panic!("dense bits")};
                    assert_eq!(bits.to_string(),expected);
                }
            }
            println!("dense authored values match native GHDL VCD at seven ticks, all4096 bits");
        }
        _ => panic!("export SOURCE STREAM | verify SOURCE FST"),
    }
}
