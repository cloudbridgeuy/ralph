pub const ANSWER_USAGE: &str = "Usage: /answer <worker> <request> <option>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerArgs {
    pub worker: String,
    pub request: String,
    pub option: String,
}

pub fn parse_answer_args(args: &str) -> Result<AnswerArgs, String> {
    let mut words = args.split_whitespace();
    match (words.next(), words.next(), words.next(), words.next()) {
        (Some(worker), Some(request), Some(option), None) => Ok(AnswerArgs {
            worker: worker.to_string(),
            request: request.to_string(),
            option: option.to_string(),
        }),
        _ => Err(ANSWER_USAGE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_words_parse_and_anything_else_is_usage() {
        assert_eq!(
            parse_answer_args(" w1 per_1  once "),
            Ok(AnswerArgs {
                worker: "w1".into(),
                request: "per_1".into(),
                option: "once".into()
            })
        );
        for args in ["", "w1", "w1 per_1", "w1 per_1 once extra"] {
            assert_eq!(parse_answer_args(args), Err(ANSWER_USAGE.to_string()));
        }
    }
}
