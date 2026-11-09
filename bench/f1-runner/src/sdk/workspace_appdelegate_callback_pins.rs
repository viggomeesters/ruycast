// Exact raw3 Main bytes; executable/data observations, not callback permission.
pub(super) struct CallbackRow {pub selector:&'static std::ffi::CStr,pub entry:usize,pub body:usize,pub notification:bool}
pub(super) struct Pin {pub role:&'static str,pub offset:usize,pub bytes:usize,pub sha:&'static str,pub words:&'static [u32]}
/// Decode only the fixed original control-entry shape. This is a dataflow
/// predicate, not permission to call a Swift body or another entry.
pub(super) fn control_entry_body(selector:&std::ffi::CStr,entry:&Pin,body:&Pin)->bool {
    let Ok(name)=selector.to_str() else{return false;};
    if !matches!(name,"openLauncher"|"openClipboard"|"openDashboard"|"openSettings"|"openMonitor"|"openConfig"|"reload")
        ||entry.role.strip_suffix("/entry")!=Some(name)||body.role.strip_suffix("/body")!=Some(name)
        ||entry.bytes!=16||entry.words.len()!=4 {return false;}
    let w=entry.words;
    if w[0]&0x9f00001f!=0x90000003||w[1]&0xff8003ff!=0x91000063
        ||w[2]&0xffe0001f!=0x52800002||w[3]&0xfc000000!=0x14000000 {return false;}
    let imm=((w[0]>>29)&3)|(((w[0]>>5)&0x7ffff)<<2);
    let signed=((imm as i32)<<11)>>11;
    let Some(page)=(entry.offset&!4095).checked_add_signed((signed as isize)<<12) else{return false;};
    let shift=if w[1]&(1<<22)!=0 {12}else{0};
    let selected=page.checked_add((((w[1]>>10)&0xfff) as usize)<<shift);
    let delta=((w[3]&0x03ffffff) as i32)<<6>>4;
    selected==Some(body.offset)&&entry.offset.checked_add(12)
        .and_then(|pc|pc.checked_add_signed(delta as isize))==Some(0x1fd314)
}
pub(super) fn control_receiver_lifetime(pin:&Pin)->bool {
    pin.role=="action_shared_CTo"&&pin.offset==0x1fd314&&pin.bytes==192&&pin.words.len()==48
        &&pin.words.get(6..9)==Some(&[0xaa0303f3,0xaa0203f7,0xaa0003f5])
        &&pin.words.get(36..48)==Some(&[
            0xaa1503e0,0x9403f9e3,0xaa0003f4,0xd63f0260,
            0x97f86fba,0xaa1403e0,0xa9447bfd,0xa9434ff4,
            0xa94257f6,0xa9415ff8,0xa8c567fa,0x1403f9d6,
        ])
}
pub(super) fn quit_receiver_lifetime(entry:&Pin,body:&Pin)->bool {
    entry.role=="quit/entry"&&entry.offset==0x1fd3f8&&entry.bytes==164&&entry.words.len()==41
        &&body.role=="quit/body"&&body.offset==0x1fd3d4&&body.bytes==36
        &&entry.words.get(4)==Some(&0xaa0003f3)&&entry.words.get(10)==Some(&0xaa0003f4)
        &&entry.words.get(31..41)==Some(&[
            0xaa1303e0,0x9403f9af,0xaa0003f3,0x97ffffd5,0x97f83879,
            0xaa1303e0,0xa9427bfd,0xa9414ff4,0xa8c357f6,0x1403f9a4,
        ])
}
/// Only the complete, pinned CTo's constant-request metadata transition.
/// This does not qualify the bridge, opaque value, destroy or other callers.
pub(super) fn notification_metadata_transition(pin:&Pin,pc:usize,target:usize)->bool {
    pin.role=="notification_shared_CTo" && pin.offset==0x1fd530 && pin.bytes==268
        && pc==0x1fd564 && target==0x2f8c24
        && pin.words.get(12..15)==Some(&[0xd2800000,0x9403edb0,0xaa0003f3])
}
/// Exact VWT-size → x9 → original stack-probe import sequence. No other
/// indirect branch or Swift value-witness invocation is covered.
pub(super) fn notification_stack_probe(pin:&Pin,pc:usize)->bool {
    pin.role=="notification_shared_CTo" && pin.offset==0x1fd530 && pin.bytes==268
        && pc==0x1fd580 && pin.words.get(15..21)==Some(&[
            0xf85f8019,0xf9402328,0xaa0803e9,0xf0000e30,0xf941ba10,0xd63f0200,
        ])
}
/// The two whole Objective-C entries supply x4 to the same compiler thunk.
/// Neither a Swift body address nor another caller may substitute for them.
pub(super) fn notification_entry_body(entry:&Pin,body:&Pin)->bool {
    entry.bytes==16 && match entry.role {
        "applicationDidFinishLaunching:/entry"=>entry.offset==0x1fcc60
            &&body.role=="applicationDidFinishLaunching:/body"&&body.offset==0x1fbc80
            &&entry.words==[0xf0ffffe4,0x91320084,0x528014c3,0x14000231],
        "applicationWillTerminate:/entry"=>entry.offset==0x1fd520
            &&body.role=="applicationWillTerminate:/body"&&body.offset==0x1fd49c
            &&entry.words==[0x90000004,0x91127084,0x52801ac3,0x14000001],
        _=>false,
    }
}
/// No opaque value crosses into Rust: original bridge writes compiler stack
/// storage; the selected body borrows it; the same VWT destroys it, then the
/// compiler restores its stack. Runtime provider/custody checks are separate.
pub(super) fn notification_value_lifetime(pin:&Pin)->bool {
    notification_metadata_transition(pin,0x1fd564,0x2f8c24)
        &&notification_stack_probe(pin,0x1fd580)&&pin.words.len()==67
        &&pin.words.get(8..12)==Some(&[0xaa0403f5,0xf81a83a3,0xaa0203f7,0xaa0003f6])
        &&pin.words.get(51..67)==Some(&[
            0xaa1803e8,0xaa1703e0,0x9403ed7c,0x97fbd694,
            0xaa0003f4,0xd63f02a0,0x97f837e3,0xf9400728,
            0xaa1803e0,0xaa1303e1,0xd63f0100,0xaa1a03e0,
            0x9403fae0,0xd10143bf,0xa9457bfd,0x17f8b1ae,
        ])
}

#[cfg(test)]
mod transition_tests {
    use super::*;
    #[test]
    fn actual_control_entries_bind_only_their_selected_bodies() {
        for row in ROWS.iter().filter(|row|!row.notification&&row.selector!=c"quit") {
            assert!(control_entry_body(row.selector,&PINS[row.entry],&PINS[row.body]));
            for other in PINS.iter().filter(|pin|pin.offset!=PINS[row.body].offset) {
                assert!(!control_entry_body(row.selector,&PINS[row.entry],other));
            }
            assert!(!control_entry_body(c"quit",&PINS[row.entry],&PINS[row.body]));
        }
    }
    #[test]
    fn control_and_quit_balanced_receiver_sequences_cannot_be_changed() {
        let action=PINS.iter().find(|p|p.role=="action_shared_CTo").unwrap();
        let quit=ROWS.iter().find(|r|r.selector==c"quit").unwrap();
        assert!(control_receiver_lifetime(action));
        assert!(quit_receiver_lifetime(&PINS[quit.entry],&PINS[quit.body]));
        assert!(!control_receiver_lifetime(&PINS[quit.entry]));
        assert!(!quit_receiver_lifetime(&PINS[quit.entry],action));
        static ACTION:std::sync::OnceLock<Vec<Vec<u32>>>=std::sync::OnceLock::new();
        for words in ACTION.get_or_init(||(6..9).chain(36..48).map(|i|{
            let mut w=action.words.to_vec();w[i]^=1;w
        }).collect()) {assert!(!control_receiver_lifetime(&Pin{words,..*action}));}
        static QUIT:std::sync::OnceLock<Vec<Vec<u32>>>=std::sync::OnceLock::new();
        let entry=&PINS[quit.entry];
        for words in QUIT.get_or_init(||[4,10].into_iter().chain(31..41).map(|i|{
            let mut w=entry.words.to_vec();w[i]^=1;w
        }).collect()) {assert!(!quit_receiver_lifetime(&Pin{words,..*entry},&PINS[quit.body]));}
    }
    #[test]
    fn actual_notification_metadata_transition_is_exclusive() {
        let pin=PINS.iter().find(|pin|pin.role=="notification_shared_CTo").unwrap();
        assert!(notification_metadata_transition(pin,0x1fd564,0x2f8c24));
        for (pc,target) in [(0x1fd560,0x2f8c24),(0x1fd568,0x2f8c24),(0x1fd564,0x2f8bf4)] {
            assert!(!notification_metadata_transition(pin,pc,target));
        }
        for other in PINS.iter().filter(|pin|pin.role!="notification_shared_CTo") {
            assert!(!notification_metadata_transition(other,0x1fd564,0x2f8c24));
        }
    }
    #[test]
    fn changed_notification_request_call_or_result_is_refused() {
        let pin=PINS.iter().find(|pin|pin.role=="notification_shared_CTo").unwrap();
        static CHANGED:std::sync::OnceLock<Vec<Vec<u32>>>=std::sync::OnceLock::new();
        let changed=CHANGED.get_or_init(|| (12..15).map(|index| {
            let mut words=pin.words.to_vec();words[index]^=1;words
        }).collect());
        for words in changed {
            let changed=Pin{words,..*pin};
            assert!(!notification_metadata_transition(&changed,0x1fd564,0x2f8c24));
        }
    }
    #[test]
    fn actual_notification_stack_probe_is_exclusive() {
        let pin=PINS.iter().find(|pin|pin.role=="notification_shared_CTo").unwrap();
        assert!(notification_stack_probe(pin,0x1fd580));
        for pc in [0x1fd578,0x1fd57c,0x1fd610,0x1fd624] {
            assert!(!notification_stack_probe(pin,pc));
        }
        for other in PINS.iter().filter(|pin|pin.role!="notification_shared_CTo") {
            assert!(!notification_stack_probe(other,0x1fd580));
        }
        static CHANGED:std::sync::OnceLock<Vec<Vec<u32>>>=std::sync::OnceLock::new();
        let changed=CHANGED.get_or_init(|| (15..21).map(|index| {
            let mut words=pin.words.to_vec();words[index]^=1;words
        }).collect());
        for words in changed {
            assert!(!notification_stack_probe(&Pin{words,..*pin},0x1fd580));
        }
    }
    #[test]
    fn complete_notification_lifetime_and_entry_body_cannot_be_rebound() {
        let pin=PINS.iter().find(|pin|pin.role=="notification_shared_CTo").unwrap();
        assert!(notification_value_lifetime(pin));
        for row in ROWS.iter().filter(|row|row.notification) {
            assert!(notification_entry_body(&PINS[row.entry],&PINS[row.body]));
            for other in PINS.iter().filter(|other|other.offset!=PINS[row.body].offset) {
                assert!(!notification_entry_body(&PINS[row.entry],other));
            }
        }
        static CHANGED:std::sync::OnceLock<Vec<Vec<u32>>>=std::sync::OnceLock::new();
        let changed=CHANGED.get_or_init(|| (8..12).chain(51..67).map(|index| {
            let mut words=pin.words.to_vec();words[index]^=1;words
        }).collect());
        for words in changed {assert!(!notification_value_lifetime(&Pin{words,..*pin}));}
    }
}
pub(super) const ROWS:[CallbackRow;10]=[
CallbackRow{selector:c"applicationDidFinishLaunching:",entry:0,body:1,notification:true},
CallbackRow{selector:c"applicationWillTerminate:",entry:2,body:3,notification:true},
CallbackRow{selector:c"openLauncher",entry:4,body:5,notification:false},
CallbackRow{selector:c"openClipboard",entry:6,body:7,notification:false},
CallbackRow{selector:c"openDashboard",entry:8,body:9,notification:false},
CallbackRow{selector:c"openSettings",entry:10,body:11,notification:false},
CallbackRow{selector:c"openMonitor",entry:12,body:13,notification:false},
CallbackRow{selector:c"openConfig",entry:14,body:15,notification:false},
CallbackRow{selector:c"reload",entry:16,body:17,notification:false},
CallbackRow{selector:c"quit",entry:18,body:19,notification:false},
];
pub(super) const PINS:&[Pin]=&[
Pin{role:"applicationDidFinishLaunching:/entry",offset:0x1fcc60,bytes:16,sha:"2041173d9caec50e56d3c4c1ac49cc8dc131dbcf641fddcb45b90a2a87a608ae",words:&[0xf0ffffe4,0x91320084,0x528014c3,0x14000231]},
Pin{role:"applicationDidFinishLaunching:/body",offset:0x1fbc80,bytes:2640,sha:"c91aa43acc4a099dfb0cbf7a651171424b26438238f8df169ce2fe32e344861d",words:&[0xa9ba6ffc,0xa90167fa,0xa9025ff8,0xa90357f6,0xa9044ff4,0xa9057bfd,0x910143fd,0xd106c3ff,0xaa1403f3,0xd2800000,0x9403faf0,0xd101c3a8,0x97f8b9c4,0xd101e3a9,0xf8100128,0x97f874fb,0xb0000e50,0xf941ba10,0xd63f0200,0x97f83e74,0xcb080128,0xd10203a9,0x97f8b9bd,0xf0000fc0,0x91042000,0xd0000861,0x91370021,0x97f821e8,0xd10163a8,0x97f8b9b3,0xd10183a9,0xf8100128,0x97f8b80f,0xb0000e50,0xf941ba10,0xd63f0200,0x910003e9,0xcb080128,0xd101a3a9,0x97f8b9ac,0xf0000fc0,0x91044000,0xd0000861,0x91372021,0x97f821d7,0xf85f8008,0x97f8b801,0xb0000e50,0xf941ba10,0xd63f0200,0x910003e9,0xcb080128,0xd10223a9,0x97f8b99e,0xd2800000,0x9403fb08,0xd10283a8,0x97f8b997,0xd102e3a9,0xf8100128,0x97f874ce,0xb0000e50,0xf941ba10,0xd63f0200,0x97f83e47,0xcb080128,0xd102a3a9,0x97f8b990,0xf0000fc0,0x91046000,0xd0000861,0x91374021,0x97f821bb,0xd10243a8,0x97f8b986,0xd102c3a9,0xf8100128,0x97f8b7e2,0xb0000e50,0xf941ba10,0xd63f0200,0x910003e9,0xcb080128,0xd10263a9,0x97f8b97f,0x97f9d34d,0xaa0003f8,0xf85f8019,0xf9402328,0xaa0803e9,0xb0000e50,0xf941ba10,0xd63f0200,0x910003e9,0x91003d0a,0x927ced4c,0xcb0c0137,0x910002ff,0xaa0803e9,0xb0000e50,0xf941ba10,0xd63f0200,0x910003e8,0xcb0c0114,0x9100029f,0xb0001081,0x91266021,0x97fffbff,0xaa0003fa,0xd2800000,0x97fb912d,0xaa0003f5,0xf9400b3b,0xaa1403e0,0x94000f5e,0xaa1403e0,0xaa1503f4,0x97fb8799,0xaa0003f6,0x90000ec0,0x9107a000,0x97f8744f,0xaa0003fc,0x94000eea,0xaa1c03e0,0x940400d4,0x90000020,0x91053000,0xaa1c03e1,0xaa1603f4,0xd2800015,0x97fb87cd,0xf0000fd9,0xb5003ab5,0xaa1c03e0,0x52800041,0x940400c6,0xf9406f3c,0x8b1c0260,0xd10323a1,0x97f83daa,0xf87c6a74,0xf83c6a76,0xd10143a8,0xf8100116,0xaa1603e0,0x940400bf,0x97f83de7,0xd2800000,0x97f9c8e3,0xaa0003f4,0xaa1703e0,0x94000f38,0x910003fa,0xb0001088,0xf944c902,0xd285a5a8,0xf2acad08,0xf2cc8c28,0xf2ecad88,0xd28e6e69,0xf2fd4009,0xa938a7a8,0x52800309,0xb0000e50,0xf941ba10,0xd63f0200,0x910003e8,0xd1008101,0x9100003f,0xd101e3a9,0xf81f0109,0xf0fff1e0,0x91323000,0x97f8f47c,0xaa0003f8,0x9100035f,0x97f928bb,0xaa0003e2,0x12000301,0xaa1703e0,0x97f928c5,0xb50034f5,0xaa0003fc,0xf0000fc8,0xf9406915,0x8b150260,0xd101e3a1,0x97f83d7a,0xf8756a74,0xf8356a7c,0xaa1c03e0,0x94040091,0x97f83db9,0xb0001088,0x91206108,0xf9400108,0x38686b88,0x370031a8,0xd2800000,0x97fd5224,0x94000ec2,0x97fd425f,0xaa0003f4,0xf0000fc8,0xf9407115,0x8b150260,0xd10243a1,0x97f83d66,0xf8756a60,0xf8356a74,0x94040078,0xd2800000,0x97fcbfb6,0x94000eb5,0x97fcb73f,0xaa0003f4,0xf0000fc8,0xf9407515,0x8b150260,0xd10383a1,0x97f83d59,0x94000ed6,0xd2800000,0x97ff1baa,0x94000eaa,0x97ff179d,0xaa0003f4,0xd0000fc8,0xf9407915,0x8b150260,0xd103e3a1,0x97f83d4e,0x94000ecb,0xaa1303f4,0x940001a9,0x940002c2,0xd0000fc0,0x91048000,0xd0000821,0x91316021,0x97f82112,0xd104a3a1,0x94040043,0xd10403a8,0xf810011c,0xaa0003fc,0xb0000808,0x3dc10500,0x3d800400,0x90000e48,0xf9454d08,0x90000e49,0xf9455129,0xf9400114,0xf9400138,0xa9026014,0xb0000f68,0xf9450500,0x9403fea4,0xaa0003f6,0xd10363a8,0xf8100100,0x90000f81,0x91398021,0xb0000f62,0x9125c042,0x97fa594e,0xaa0003f5,0xf940139a,0xaa1403e0,0x9403fe9e,0xaa1803e0,0x9403fe9c,0xd10383a8,0xf8100100,0xaa1a03e0,0x9403fe98,0xaa0003f7,0x90000f68,0xf943bd01,0xaa1603e0,0x9403fe87,0xaa1d03fd,0x9403fe97,0xaa0003f4,0x90000f68,0xf943c501,0x9403fe81,0xaa1d03fd,0x9403fe91,0xaa0003fa,0x97f83d24,0xd102a3a8,0xf8500119,0xaa1903e8,0xaa1703e0,0xd2800001,0xaa1a03f4,0x9403fa17,0x97f83e42,0xaa1503f4,0xd10343a8,0xf8100115,0x9403f9d3,0xaa0003fa,0xd10103a8,0xf8100100,0xd2800000,0x9403f9bf,0xaa0003e3,0xd10323a8,0xf8100100,0xd10223a8,0xf850011b,0xaa1b03e0,0x52800021,0x52800022,0x97f8396a,0xd0000fc0,0x9104c000,0xb0000e41,0xf943dc21,0xb0000e42,0xf943d842,0x97f8b665,0xaa0003f6,0xd103a3a8,0xf8100100,0x94000cc1,0xaa0003e5,0xd10303a8,0xf8100100,0xd10503a0,0xd10263a8,0xf8500115,0xaa1503e8,0xaa1b03e1,0xd10283a9,0xf8500138,0xaa1803e2,0xaa1403e3,0xaa1603e4,0xaa1903f4,0x9403f534,0xd0000fc1,0x91044021,0xb0000862,0x91372042,0xaa1b03e0,0x97f9fca9,0x97f83e0f,0xd102e3a8,0x97f905d5,0xd102e3a9,0xf8100128,0xaa1903e0,0xaa1803e1,0xd63f0100,0xf0000ea0,0x9107a000,0x97f8735f,0xaa0003f6,0x94000dfa,0xd0000fc0,0x91050000,0xd0000fc1,0x91046021,0xb0000862,0x91374042,0x90000e43,0xf946d863,0x97f9fc22,0xaa0003e3,0xd103c3a8,0xf8100100,0xf0000000,0x9113f000,0xaa1603e1,0xd10243a8,0xf850011a,0xaa1a03e2,0xaa1503f4,0x9403f513,0xaa0003f4,0x97f87406,0xd102c3a8,0x97f905b3,0xd102c3a9,0xf8100128,0xaa1503e0,0xaa1a03e1,0xd63f0100,0xd0000fc8,0xf9408116,0xd103e3a8,0xf8100116,0x8b160260,0xd10503a1,0x97f903a1,0x8b160260,0x9403f4d7,0x94000e1b,0x97f83cec,0x97f83ce6,0xaa1c03e0,0x9403ffc1,0xd10383a8,0xf8500100,0x9403fe14,0xaa0003f7,0xaa1c03f4,0xd10403a8,0xf850011c,0x97fbfa6f,0x97f90445,0x90000f68,0xf943bd01,0xd10363a8,0xf8500100,0x9403fdfd,0xaa1d03fd,0x9403fe0d,0xaa0003f4,0x90000f68,0xf943c501,0x9403fdf7,0xaa1d03fd,0x9403fe07,0xaa0003f6,0x97f83c9a,0xaa1903e8,0xaa1703e0,0xd2800001,0xaa1603f4,0x9403f98f,0x97f83c96,0xd10343a8,0xf8500114,0x9403f94c,0xaa0003f6,0xd10103a8,0x94000dc4,0xd10503a0,0xaa1503e8,0xaa1b03e1,0xaa1803e2,0xaa1403e3,0xd103a3a9,0xf8500124,0xd10303a9,0xf8500125,0xaa1903f4,0x9403f4c7,0xaa1b03e0,0xd0000fc1,0x91044021,0xb0000862,0x91372042,0x97f9fc3c,0x97f83c7e,0xaa1903e0,0xaa1803e1,0xd102e3a8,0xf8500108,0xd63f0100,0xf0000ea0,0x9107a000,0x97f872f4,0xaa0003f6,0x94000d8f,0xf0000000,0x9125c000,0xaa1603e1,0xaa1a03e2,0xd103c3a8,0xf8500103,0xaa1503f4,0x9403f4b4,0xaa0003f4,0x97f873a7,0xaa1503e0,0xaa1a03e1,0xd102c3a8,0xf8500108,0xd63f0100,0xd103e3a8,0xf8500119,0x8b190260,0xd10503a1,0x97f90346,0x8b190260,0x9403f47c,0x94000dc0,0x97f83c91,0x97f83c8b,0x90001088,0x9121c108,0xf9400108,0xf8686b94,0xd2800000,0x97f8ffdd,0xaa0003f6,0x90fff201,0x910f2021,0xf0000f60,0x91284000,0xd0000802,0x910fa042,0x97f8b5ad,0xaa0003e1,0xaa1603e0,0x9403f46e,0xaa0003f6,0xd10103a8,0xf8100100,0xd10203a8,0xf850011a,0xaa1a03e8,0x52800040,0x9403f8ef,0xd10343a8,0xf8500115,0xaa1503f4,0x9403f8f7,0xaa0003f7,0xd10123a8,0x94000d6f,0xd2800000,0x9403f463,0xaa0003e3,0x90000e45,0xf946f8a5,0xd10523a1,0xd10503b4,0xd101a3a8,0xf8500118,0xaa1803e8,0xaa1a03e0,0xaa1b03e2,0xaa1503e4,0xd10303a9,0xf8500126,0x9403f470,0xaa1b03e0,0xd0000fc1,0x91044021,0xb0000862,0x91372042,0x97f9fbe2,0x97f83c59,0xd101e3a8,0x97f9050e,0xaa1a03e0,0xd101c3a9,0xf8500121,0xd63f0100,0x97f8735a,0xf0000ea0,0x9107a000,0x97f87298,0xaa0003f5,0x94000d33,0xd0000fc0,0x91054000,0xd0000fc1,0x91042021,0xb0000862,0x91370042,0x90000e43,0xf946d063,0x97f9fb5b,0xaa0003e3,0xf0000000,0x91141000,0xaa1503e1,0xd10163a8,0xf8500116,0xaa1603e2,0xaa1803f4,0x9403f44e,0xaa0003f4,0x97f8b699,0xd10183a8,0x97f904ee,0xaa1803e0,0xaa1603e1,0xd63f0100,0x8b190260,0xd10503a1,0x97f902e2,0x8b190260,0x9403f418,0x94000d5c,0x97f83c28,0x97f83c9b,0xd10143a8,0xf8500100,0x9403fef5,0xd10143bf,0xa9457bfd,0x17f8b5c3,0x97f8732d,0x97f83c93,0x14000003,0xd10143a8,0xf850011c,0x97f83c8f,0xd0000fc8,0xf9406914,0x8b140260,0xd101e3a1,0x97f83b92,0xf8746a74,0xb4000094,0x97f87285,0x97f9463a,0x97f83c11,0xf9406f36,0x8b160260,0xd10243a1,0x97f83bc9,0xd2800000,0xf8766a74,0xb40000b4,0x97f8727b,0x97fb8d95,0x97f83c07,0xf8766a60,0xf8366a7f,0x9403fed5,0xd10263a1,0xd102c3a2,0x97f9d27c,0xa97503b4,0xf85603a1,0x9403fa25,0xaa0103f3,0xa93207a0,0xaa0103e0,0x9403fe50,0xd10383b4,0x52800140,0xd2fc2001,0x9403f7d7,0x97f83ded,0xa97253b3,0x9403f3d5,0xaa0003f5,0xaa1303e0,0xaa1403e1,0x9403f79d,0xaa0003f3,0x97f83be6,0x91008260,0xaa1503e1,0x9403fc75,0x97f8b6eb,0x52800020,0x9403fc54]},
Pin{role:"applicationWillTerminate:/entry",offset:0x1fd520,bytes:16,sha:"b688f4286000ffbf5f719a0daedc6e8a9909ef8ce230f2884b97e82da7edc06c",words:&[0x90000004,0x91127084,0x52801ac3,0x14000001]},
Pin{role:"applicationWillTerminate:/body",offset:0x1fd49c,bytes:132,sha:"c4f0b80ab4a1f5f8eec27f4fa8cccdcf1eb1226c91e21a5c946a44056bfe55ea",words:&[0xd10183ff,0xa90357f6,0xa9044ff4,0xa9057bfd,0x910143fd,0xaa1403f3,0xb0000fc8,0xf9406914,0x8b140260,0x910063e1,0x97f837e5,0xf8746a74,0xb4000094,0x97f86ed8,0x97f9428d,0x97f83864,0xb0000fc8,0xf9406d15,0x8b150260,0x910003e1,0x97f8381b,0xf8756a74,0xb40000d4,0x97f86ece,0x97fb89e8,0x97f8385a,0xf8756a60,0x14000002,0xd2800000,0xf8356a7f,0x9403fb26,0xa9457bfd,0x17f9d055]},
Pin{role:"openLauncher/entry",offset:0x1fcf00,bytes:16,sha:"d88a50c68a20008f03b44275e58e6f4ef08a36eb98c0371182afe170902de25a",words:&[0x90000003,0x913a0063,0x528019c2,0x14000102]},
Pin{role:"openLauncher/body",offset:0x1fce80,bytes:128,sha:"9603f5b80b4a5fb517bfb53511886cac7e1bb39e422b5bd13d10e233a5c911f6",words:&[0xd10143ff,0xa9034ff4,0xa9047bfd,0x910103fd,0xaa1403f3,0xd0000fc8,0xf9407114,0x8b140260,0x910063e1,0x97f8396d,0xf8746a74,0xb4000134,0x97f87060,0xd28dec60,0xf2adada0,0xf2cdcc20,0xf2ee6c80,0xd2fd0001,0x97fd40a7,0x97f839e7,0xd0000fc8,0xf9406914,0x8b140260,0x910003e1,0x97f8395e,0xf8746a74,0xb4000094,0x97f87051,0x97f92aeb,0x97f839dd,0xa9447bfd,0x17fcd5a3]},
Pin{role:"openClipboard/entry",offset:0x1fcf64,bytes:16,sha:"372ac1bec2719e75c53ca9818ebcc6c9d95fe047acf95a38c63300315f502ff8",words:&[0x90000003,0x913c4063,0x528019e2,0x140000e9]},
Pin{role:"openClipboard/body",offset:0x1fcf10,bytes:84,sha:"d0622f0a333f13baf1da1a4023cc547549a81f922dfb3bac3c8743f08a258140",words:&[0xd10103ff,0xa9024ff4,0xa9037bfd,0x9100c3fd,0xd0000fc8,0xf9407113,0x97f9cd5e,0xf8736a94,0xb4000154,0x97f8703f,0xd28d8c60,0xf2ae0d20,0xf2cdec40,0xf2ee4c20,0xd2800c81,0xf2fd2001,0x97fd4085,0x97f839c5,0xa9437bfd,0xa9424ff4,0x17fc2868]},
Pin{role:"openDashboard/entry",offset:0x1fcfe8,bytes:16,sha:"6831f8144af957f767d45a6705491510c5bf6ef3cf37f71b9df981ed321ee64c",words:&[0x90000003,0x913dd063,0x52801a02,0x140000c8]},
Pin{role:"openDashboard/body",offset:0x1fcf74,bytes:116,sha:"d1e85250e29ba0fd367eacd8eefd572937dcce7dd0870c85b522180a16b73ce5",words:&[0xd10183ff,0xa90357f6,0xa9044ff4,0xa9057bfd,0x910143fd,0xd0000fc8,0xf9406913,0x8b130280,0x910063e1,0x97f83930,0xf8736a88,0xb4000208,0x90001089,0x91230129,0xf9400129,0x8b090113,0x910003e1,0xaa1303e0,0x97f83927,0xf9400275,0xb40000f5,0xf9400674,0x97f87019,0xd63f02a0,0xaa1503e0,0xaa1403e1,0x97f838c0,0xa9457bfd,0x17f9d1a3]},
Pin{role:"openSettings/entry",offset:0x1fd038,bytes:16,sha:"fdd0f1cd81625fe8ee70d015c98a687316f9e1b8ca1226721b5ec439a97830f4",words:&[0xf0ffffe3,0x913fe063,0x52801a22,0x140000b4]},
Pin{role:"openSettings/body",offset:0x1fcff8,bytes:64,sha:"d649ebd3164ca8ad5d3f2926cf75ce583d7f91e32c263e4b7f9ccc71e2125e94",words:&[0xd10103ff,0xa9024ff4,0xa9037bfd,0x9100c3fd,0xb0000fc8,0xf9407513,0x97f9cd24,0xf8736a80,0xb40000a0,0x9403fac6,0xaa0003f4,0x97fcb431,0x97f8395e,0xa9437bfd,0xa9424ff4,0x17fc2833]},
Pin{role:"openMonitor/entry",offset:0x1fd0b8,bytes:16,sha:"2c398bb729f0e77381b9d8272df8b259b5d284b336de34b4366637b32a5e763b",words:&[0x90000003,0x91012063,0x52801a42,0x14000094]},
Pin{role:"openMonitor/body",offset:0x1fd048,bytes:112,sha:"bfdaaed323c32d6471edb3c8e8c7bcd36e9ad582bbc9b82526ba21d924771684",words:&[0xd10143ff,0xa9034ff4,0xa9047bfd,0x910103fd,0xaa1403f3,0xb0000fc8,0xf9407114,0x8b140260,0x910063e1,0x97f838fb,0xf8746a74,0xb4000094,0x97f86fee,0x97fd42ea,0x97f8397a,0xb0000fc8,0xf9407914,0x8b140260,0x910003e1,0x97f838f1,0xf8746a60,0xb40000a0,0x9403faa5,0xaa0003f4,0x97ff1433,0x97f8393d,0xa9447bfd,0x17fcd535]},
Pin{role:"openConfig/entry",offset:0x1fd1a8,bytes:16,sha:"b875dda4859fa4cadb993f2e6287df203546eb0252fba323da4c4c057e97f8e9",words:&[0x90000003,0x91032063,0x52801a62,0x14000058]},
Pin{role:"openConfig/body",offset:0x1fd0c8,bytes:224,sha:"e950012b4361c9797bd812c0c8775e2def2c49a3acf3cdda20f88c2ef0418d09",words:&[0xa9bc5ff8,0xa90157f6,0xa9024ff4,0xa9037bfd,0x9100c3fd,0xd10083ff,0x97f9ce8a,0xaa0003f3,0xf85f8018,0xf9402308,0xaa0803e9,0xf0000e30,0xf941ba10,0xd63f0200,0x97f83967,0xcb080135,0x910002bf,0xb0000fc8,0xf9406916,0x8b160280,0xd10123a1,0x97f838cf,0xf8766a96,0xb40003d6,0x90000f68,0xf9450500,0x9403fa7b,0xaa0003f4,0xaa1603e0,0x9403fc22,0xf0000f48,0xf943bd01,0xaa1403e0,0x9403fa6e,0xaa1d03fd,0x9403fa7e,0xaa0003f7,0xaa1503e8,0xaa1603f4,0x97f92419,0xaa1503f4,0x9403ef3e,0xaa0003f4,0xf9400708,0x97f90154,0xf0000f48,0xf943e901,0xaa1703e0,0xaa1403e2,0x9403fa5e,0x97f87043,0x97f8393a,0x97f83902,0xd100c3bf,0xa9437bfd,0x17f870c4]},
Pin{role:"reload/entry",offset:0x1fd304,bytes:16,sha:"a7dc03dd48a1588eeacedbf0b37941536f7f130872cd70664a11585b090e912d",words:&[0x90000003,0x9106e063,0x52801a82,0x14000001]},
Pin{role:"reload/body",offset:0x1fd1b8,bytes:332,sha:"bcad32ab92528a1b60095a273203f173e9bbb1245f6c54c056186b5e9b3195e5",words:&[0xd10743ff,0xa9195ff8,0xa91a57f6,0xa91b4ff4,0xa91c7bfd,0x910703fd,0xaa1403f3,0xb0000fc8,0xf9406914,0x8b140260,0x910383e1,0x97f8389d,0xf8746a74,0xb4000594,0x910123e0,0x97f83652,0xd10323a0,0x910123e1,0x52801222,0x9403fa1c,0x97f86f8a,0xd10323a2,0xd2800000,0xd2fe0001,0xd2800003,0xd2800004,0xd2800015,0x97f92edf,0xaa1403e0,0xb4000375,0x9403fbdf,0xb0000fc8,0xf9407114,0x8b140260,0x9100a3e1,0x97f83885,0xf8746a73,0xb40004d3,0x910083e1,0x910023e2,0x97f9cf7f,0xa940dbf4,0xf9400ff7,0x97f90300,0xaa1603e0,0xaa1703e1,0x9403f725,0xaa0103f6,0x52800002,0xaa1303f4,0x97fd4326,0xaa1503e0,0x9403fb78,0x97f8b3f9,0x97f839f4,0x14000016,0x9403fbc5,0xb0000fc8,0xf9407114,0x8b140260,0x910123e1,0x97f8386b,0xf8746a74,0xb40001d4,0x90000b48,0x911b4108,0xd1008113,0x97f86f5b,0xb2410261,0xd28002c0,0xf2fa0000,0x52800022,0x97fd4310,0x97f838e3,0x14000003,0xaa1503e0,0x9403fb60,0xa95c7bfd,0xa95b4ff4,0xa95a57f6,0xa9595ff8,0x910743ff,0xd65f03c0]},
Pin{role:"quit/entry",offset:0x1fd3f8,bytes:164,sha:"eb510ab8876a20688af1314e28a2523d865572596e4d3c5a2031925d1e2e99ef",words:&[0xa9bd57f6,0xa9014ff4,0xa9027bfd,0x910083fd,0xaa0003f3,0x97f8392f,0xaa0003f5,0xd0000e56,0xf94406d6,0x97f83905,0xaa0003f4,0xb0000fa0,0x911d4000,0xd0000e42,0xf9440842,0xaa1603e1,0x97f8b1bc,0xaa0003e1,0xaa1503e0,0x9403f4cd,0xaa0003f5,0xaa0103f6,0x9403fb84,0x37000100,0xf0000bc0,0x91200000,0x9400099c,0x52801aa3,0xaa1503e4,0xaa1603e5,0x9403fb7f,0xaa1303e0,0x9403f9af,0xaa0003f3,0x97ffffd5,0x97f83879,0xaa1303e0,0xa9427bfd,0xa9414ff4,0xa8c357f6,0x1403f9a4]},
Pin{role:"quit/body",offset:0x1fd3d4,bytes:36,sha:"2cfe520d5e384dec626fb213e983f5fdb1c25b6d9fbd2949ee90e7848c8d2f98",words:&[0xf0000e28,0xf944f108,0xf9400100,0xb40000a0,0xf0000f48,0xf9440501,0xd2800002,0x1403f9c5,0xd4200020]},
Pin{role:"notification_shared_CTo",offset:0x1fd530,bytes:268,sha:"06887bfddee21c07603dfbf8d581d71e78ff91b7b280d05879602303f4dbfaea",words:&[0xa9ba6ffc,0xa90167fa,0xa9025ff8,0xa90357f6,0xa9044ff4,0xa9057bfd,0x910143fd,0xd10043ff,0xaa0403f5,0xf81a83a3,0xaa0203f7,0xaa0003f6,0xd2800000,0x9403edb0,0xaa0003f3,0xf85f8019,0xf9402328,0xaa0803e9,0xf0000e30,0xf941ba10,0xd63f0200,0x97f83846,0xcb080138,0x9100031f,0x97f838ce,0xaa0003fb,0xd0000e5c,0xf944079c,0x97f838a4,0xaa0003fa,0xb0000fa0,0x911d4000,0xd0000e42,0xf9440842,0xaa1c03e1,0x97f8b15b,0xaa0003e1,0xaa1a03f4,0xaa1b03e0,0x9403f46b,0xaa0003f4,0xaa0103fb,0x9403fb22,0x37000100,0xf0000bc0,0x91200000,0x9400093a,0xf85a83a3,0xaa1403e4,0xaa1b03e5,0x9403fb1d,0xaa1803e8,0xaa1703e0,0x9403ed7c,0x97fbd694,0xaa0003f4,0xd63f02a0,0x97f837e3,0xf9400728,0xaa1803e0,0xaa1303e1,0xd63f0100,0xaa1a03e0,0x9403fae0,0xd10143bf,0xa9457bfd,0x17f8b1ae]},
Pin{role:"action_shared_CTo",offset:0x1fd314,bytes:192,sha:"580781ad15e6dacb7ac532ee46817fb5a9721dc7c20bda08b19e3a50a891dbe1",words:&[0xa9bb67fa,0xa9015ff8,0xa90257f6,0xa9034ff4,0xa9047bfd,0x910103fd,0xaa0303f3,0xaa0203f7,0xaa0003f5,0x97f83964,0xaa0003f8,0xd0000e59,0xf9440739,0x97f8393a,0xaa0003f6,0xb0000fa0,0x911d4000,0xd0000e42,0xf9440842,0xaa1903e1,0x97f8b1f1,0xaa0003e1,0xaa1603f4,0xaa1803e0,0x9403f501,0xaa0003f4,0xaa0103f8,0x9403fbb8,0x37000100,0xf0000bc0,0x91200000,0x940009d0,0xaa1703e3,0xaa1403e4,0xaa1803e5,0x9403fbb3,0xaa1503e0,0x9403f9e3,0xaa0003f4,0xd63f0260,0x97f86fba,0xaa1403e0,0xa9447bfd,0xa9434ff4,0xa94257f6,0xa9415ff8,0xa8c567fa,0x1403f9d6]},
Pin{role:"ivar_destructor_CTo",offset:0x1fd784,bytes:124,sha:"7ecc29b9fb0f23717334abc85f9abd767566b99ec5482513204190905d542dee",words:&[0xa9be4ff4,0xa9017bfd,0x910043fd,0xaa0003f3,0xb0000fc8,0xf9406908,0xf8686800,0x9403fa83,0xb0000fc8,0xf9406d08,0xf8686a60,0x9403fa7f,0xb0000fc8,0xf9407108,0xf8686a60,0x9403fa7b,0xb0000fc8,0xf9407508,0x940008e6,0xb0000fc8,0xf9407908,0x940008e3,0xb0000fc8,0xf9407d08,0x940008e0,0xb0000fc8,0xf9408108,0xf8686a60,0xa9417bfd,0xa8c24ff4,0x1403f9eb]},
Pin{role:"notification_dynamic_stack_align",offset:0xb69c,bytes:16,sha:"bf768053eb4843f7efeac0573001efd27858e6fe2ad2a9200449b8339fb0c4a5",words:&[0x910003e9,0x91003d08,0x927ced08,0xd65f03c0]},
Pin{role:"notification_retain_self_tail",offset:0xf3058,bytes:8,sha:"b81d6bc50b8d2442f59c09b14a2b989e98b6f052dba8a41f7f7f584f3e7d4abf",words:&[0xaa1603e0,0x140822b6]},
Pin{role:"notification_postbody_objc_release_tail",offset:0xb5a0,bytes:8,sha:"62b51538082e9cc52ff59387782f986dfaa6dfd632b106976c957d86d81f1ce4",words:&[0xaa1403e0,0x140bc161]},
Pin{role:"destructor_objc_field_release_tail",offset:0x1ffb64,bytes:8,sha:"31d77d319b9301b1422adcaa15c611a1123e564ec3a731eec4d4ec78873c77b7",words:&[0xf8686a60,0x1403eff0]},
Pin{role:"notification_restore_registers",offset:0x29cf0,bytes:24,sha:"d55d607138dd9d6cb641359938e4866bdfd51135e1b37aa0a7390742dfef3006",words:&[0xa9444ff4,0xa94357f6,0xa9425ff8,0xa94167fa,0xa8c66ffc,0xd65f03c0]},
Pin{role:"quit_release_actor",offset:0xb668,bytes:8,sha:"c290782f8e7c8a71e4a1736e94a921fbd01ec6040124767e5beb153a0154b31c",words:&[0xaa1403e0,0x140bc2d0]},
Pin{role:"control_release_actor",offset:0x1929c,bytes:8,sha:"62708fc6c53bf9049a7c8e400f44fbf9dea76906e4a0757b82fca241005da02d",words:&[0xaa1603e0,0x140b8bc3]},
];
