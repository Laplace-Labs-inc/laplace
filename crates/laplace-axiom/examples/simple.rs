use laplace_axiom::oracle::OracleVerdict;
use laplace_axiom::oracle::{AxiomOracle, OracleConfig};
use laplace_axiom::simulation::VirtualEnvPlugin;
use laplace_axiom::simulation::{NullObserver, TwinSimulatorBuilder};
use laplace_core::domain::memory::{Address, CoreId, Value};
use laplace_core::domain::memory::{MemoryBackend, MemoryOp};
use laplace_core::domain::scheduler::ThreadId;

pub struct ChaosInjector;

impl VirtualEnvPlugin for ChaosInjector {
    fn inject_memory_op(
        &mut self,
        _tick: u64,
        _memory: &mut dyn MemoryBackend,
    ) -> Option<MemoryOp> {
        // 필요하다면 여기서 고객의 메모리를 직접 오염시켜 존재할 수 없는 상태를 강제한다.
        None
    }

    fn inject_schedule_override(&mut self, tick: u64) -> Option<ThreadId> {
        // 운명 조작: 10번째 틱에서 강제로 1번 스레드를 깨워 치명적인 락(Lock) 경합을 유도한다.
        // 고객의 코드는 완벽한 타이밍에 날아온 이 스나이퍼 샷을 결코 피할 수 없다.
        if tick == 10 {
            Some(ThreadId::new(1))
        } else {
            None
        }
    }

    fn name(&self) -> &'static str {
        "Axiom-Chaos-Injector-v1"
    }
}

fn main() {
    let mut sim = TwinSimulatorBuilder::new()
        .cores(4)
        .scheduler_threads(4)
        .finalize()
        .inject(ChaosInjector)
        .observe(NullObserver)
        .build();

    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(42))
        .unwrap();
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(1), Value::new(99))
        .unwrap();

    let report = sim.run_until_idle();

    assert_eq!(
        report.events_processed, 2,
        "이벤트가 누락되었다면 너의 실수다."
    );
    assert!(sim.is_idle(), "잔여 이벤트가 없어야 한다.");

    // 메인 메모리에 새겨진 결과를 렌더링한다.
    println!(
        "Simulation complete — {} events processed, {} steps",
        report.events_processed, report.steps_executed
    );
    println!(
        "addr[0] = {}, addr[1] = {}",
        sim.memory().read_main_memory(Address::new(0)).0,
        sim.memory().read_main_memory(Address::new(1)).0,
    );

    let oracle = AxiomOracle::new(OracleConfig {
        max_depth: 200,                      // 탐색할 평행우주의 물리적 깊이 한계
        output_dir: "./reports".to_string(), // 사형 선고문(.ard)이 덤프될 위치
        ..OracleConfig::default()
    });

    let verdict = oracle.run_exhaustive(
        "chaos_target",
        &mut sim,
        200,
        |_thread, _pc| {
            // [타겟 오퍼레이션 생략: 고객의 비즈니스 로직 매핑]
            None
        },
        |sim| {
            // 매 스텝마다 고객의 메모리(0번 주소 - 예: 잔고, 티켓 수)를 훔쳐본다.
            if let Some(val) = sim.read_memory(Address::new(0)) {
                if val.0 > 100 {
                    // 고객이 '절대 100을 넘을 수 없다'고 호언장담한 불변성
                    // 단두대 작동. 오라클은 이 문자열을 받는 즉시 탐색을 중단하고 사형을 집행한다.
                    return Some("불변성 파괴: 값이 100을 초과함. 시스템 붕괴.".to_string());
                }
            }
            None // 아직은 숨을 쉬도록 허락한다.
        },
    );

    match verdict {
        OracleVerdict::Clean => {
            println!("제국의 심판 결과: 무죄. 이 코드는 완벽하다.");
        }
        OracleVerdict::BugFound {
            ard_path,
            description,
        } => {
            println!("사형 선고: {}", description);
            println!("CYA 증거물 확보 완료. 포렌식 파일 경로: {}", ard_path);
            // 집행관은 이 .ard 파일을 챙겨서 고객의 CFO에게 청구서를 내밀면 된다.
        }
    }
}
