use super::args::Options;
use crate::text;
use serde_json::Value;

pub(super) fn output(result: &Value, o: &Options) {
    if o.flag("csv") {
        println!("id,title,price,currency,city,url");
        for v in result["items"].as_array().into_iter().flatten() {
            println!(
                "{}",
                ["id", "title", "price", "currency", "city", "url"]
                    .map(|k| format!("\"{}\"", text(&v[k]).replace('"', "\"\"")))
                    .join(",")
            );
        }
    } else if o.flag("quiet") {
        for v in result["items"].as_array().into_iter().flatten() {
            println!("{}", text(&v["url"]));
        }
    } else {
        println!("{result}");
    }
}
