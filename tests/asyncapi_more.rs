use sanshain_service::asyncapi::split_asyncapi;

fn v2_base(channels: &str) -> String {
    format!(
        "asyncapi: '2.6.0'\ninfo: {{ title: T, version: 1.0.0 }}\nchannels:\n{}\n",
        channels
    )
}

fn v3_base(operations: &str, channels: &str) -> String {
    format!(
        "asyncapi: '3.0.0'\ninfo: {{ title: T, version: 1.0.0 }}\noperations:\n{}\nchannels:\n{}\n",
        operations, channels
    )
}

// 1. v2 publish and subscribe create two specs
#[test]
fn v2_publish_and_subscribe() {
    let y = v2_base("  user.signedup:\n    publish: {}\n    subscribe: {}\n");
    let specs = split_asyncapi(&y).unwrap();
    let ops: Vec<_> = specs.iter().map(|s| s.operation.as_str()).collect();
    assert!(ops.contains(&"PUB"));
    assert!(ops.contains(&"SUB"));
}

// 2. v3 send action mapped to PUB
#[test]
fn v3_send_maps_to_pub() {
    let ops = "  SendSignup:\n    action: send\n    channel: { $ref: '#/channels/UserSignup' }\n";
    let chs = "  UserSignup: { address: 'user.signedup' }\n";
    let y = v3_base(ops, chs);
    let specs = split_asyncapi(&y).unwrap();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].operation, "PUB");
}

// 3. v3 receive action mapped to SUB
#[test]
fn v3_receive_maps_to_sub() {
    let ops =
        "  ReceiveSignup:\n    action: receive\n    channel: { $ref: '#/channels/UserSignup' }\n";
    let chs = "  UserSignup: { address: 'user.signedup' }\n";
    let y = v3_base(ops, chs);
    let specs = split_asyncapi(&y).unwrap();
    assert_eq!(specs[0].operation, "SUB");
}

// 4. v3 unknown action errors
#[test]
fn v3_unknown_action_errors() {
    let ops = "  Weird:\n    action: dance\n    channel: { $ref: '#/channels/C' }\n";
    let chs = "  C: { address: 'topic' }\n";
    let y = v3_base(ops, chs);
    let err = split_asyncapi(&y).err().unwrap();
    assert!(err.contains("Unknown operation action"));
}

// 5. v3 missing channel ref errors
#[test]
fn v3_missing_channel_ref_errors() {
    let ops = "  O:\n    action: send\n"; // no channel.$ref
    let y = v3_base(ops, "");
    let err = split_asyncapi(&y).err().unwrap();
    assert!(err.contains("missing channel.$ref"));
}

// 6. invalid YAML returns parse error
#[test]
fn invalid_yaml_errors() {
    let y = "asyncapi: '2.6.0'\nchannels: [oops"; // broken
    let err = split_asyncapi(y).err().unwrap();
    assert!(err.to_lowercase().contains("parse"));
}
