// 仅使用已结束调用计算失败比例，防止把等待或取消误认为新版本故障。
fn invoke(input, context) {
    if input.minimum_completed < 1 || input.failure_ratio <= 0 || input.failure_ratio > 1 {
        throw "观察阈值无效";
    }
    let head = call_capability("/v1/runtime/enhancements/head", #{});
    if head.revision != input.revision || head.generation != input.expected_generation {
        throw "观察版本已变化，必须重新审阅";
    }
    let observations = call_capability("/v1/runtime/enhancements/observe", #{revision: head.revision, generation: head.generation});
    for sample in observations {
        if sample.path == input.path {
            let completed = sample.succeeded + sample.failed;
            if completed >= input.minimum_completed && sample.failed.to_float() / completed.to_float() >= input.failure_ratio {
                let restored = call_capability("/v1/runtime/enhancements/rollback", #{
                    revision: input.previous_revision, expected_generation: head.generation
                });
                return #{rolled_back: true, head: restored};
            }
        }
    }
    #{rolled_back: false, head: head}
}
