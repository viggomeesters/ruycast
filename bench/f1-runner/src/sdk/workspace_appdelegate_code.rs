// Fixed same-build raw3 DATA. No function-pointer/ARC permission.
#[derive(Clone,Copy)]
pub(super) enum Role {WholeObjcInit,SwiftInitStoresAndSuper,CxxDestroySevenFields,CallerConstructAndPublish,Secondary}
pub(super) struct Pin {pub role:Role,pub offset:usize,pub bytes:usize,pub sha:&'static str}
pub(super) const CODE:[Pin;18]=[
    Pin {role:Role::WholeObjcInit,offset:0x1fd6f8,bytes:140,sha:"6d4e0125f92dfb2995212d7d2ffe52db5aedac7fdad253cda0a7c0fca312578b"},
    Pin {role:Role::SwiftInitStoresAndSuper,offset:0x1fd63c,bytes:188,sha:"d81d2902a804b31b6dc88fb50a752d891961549ddafd1e424b65b49349e9badf"},
    Pin {role:Role::CxxDestroySevenFields,offset:0x1fd784,bytes:272,sha:"39b2abad0eab5f911dc77ddc7da528a652c25b31ae35fd0678db86e08fe70d84"},
    Pin {role:Role::CallerConstructAndPublish,offset:0x1f8694,bytes:40,sha:"e3380588f3702615f5335355c6a5b3c670144e3bbe4341c1436d69af32df8fc3"},
    Pin {role:Role::Secondary,offset:0xb8c8,bytes:8,sha:"3b6361c308840bbd6a669aad19157e268d7f15c6b204b75723f5163003b18372"},
    Pin {role:Role::Secondary,offset:0xb830,bytes:8,sha:"5121800bbb9bee0ef35fb5568ebdaae66d7af77487d45b789c09efdb9440a69f"},
    Pin {role:Role::Secondary,offset:0x2a128,bytes:8,sha:"271c20c3fbd280e9e837350ec0e7c22abcf1e1b728ed50b385598443ae0fa103"},
    Pin {role:Role::Secondary,offset:0x9b7a8,bytes:16,sha:"f21d9192ce83fbcf3a6ba5b48e231017825eb63cdc3129ed5e678f783820e708"},
    Pin {role:Role::Secondary,offset:0x29dc0,bytes:12,sha:"885b9d37af6a2abaef5fe0ed9858e7a089bc19135540748312a8cd903db453a3"},
    Pin {role:Role::Secondary,offset:0x1ffad0,bytes:12,sha:"f8d71fa0fb4da6e1b59c3e1ddad4c646e0047e6a9e4fba8e7f60333c852932cb"},
    Pin {role:Role::Secondary,offset:0x29db4,bytes:12,sha:"a957e482e38160b09e87c345eb674c8f90f6e7f48d45e75722aadc223b03635b"},
    Pin {role:Role::Secondary,offset:0x29ffc,bytes:8,sha:"b48d758f4e73ac72cca7c53259d2b2bf297f59f2210ad59079c805db123a44f4"},
    Pin {role:Role::Secondary,offset:0x29b28,bytes:52,sha:"d4da500b79a082fea5848f74928b0f6efa7a8f46de8c8a030d74d5e57cbb4948"},
    Pin {role:Role::Secondary,offset:0x2a6b0,bytes:12,sha:"2250fb28cb2f50d431b1616e2698fcd30873fc58141c4a7ef882ed7cc462f22d"},
    Pin {role:Role::Secondary,offset:0xba8c,bytes:8,sha:"4ccf46ba787b1ef23f60679596cc55c190cfae5e3665f7170336cdd6b13f2497"},
    Pin {role:Role::Secondary,offset:0x12a67c,bytes:32,sha:"2f316f8dc60a62171e8db0ea75a17a9d303e2296e8a9a9234e32cc6ab00ae32f"},
    Pin {role:Role::Secondary,offset:0xa850,bytes:36,sha:"2ef52d43c3959ceb6624c41a3aad8597d4383e2a35e421eb63bf971f25ad52a3"},
    Pin {role:Role::Secondary,offset:0x1fd894,bytes:32,sha:"c9cc4cc656ee644e73512fdf3f3a991b0713abb5ad7205994a9051e973469d1e"},
];
