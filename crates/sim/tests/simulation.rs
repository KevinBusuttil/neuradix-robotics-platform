//! Behavioural and determinism tests for the depth simulation.

use neuradix_sim::{
    Controller, DepthPlant, DepthSensor, PlantParams, PlantState, ProportionalController,
    SensorParams, SimError, Simulation, StepContext,
};
use neuradix_time::{ClockDomain, Duration, Timestamp};

fn start() -> Timestamp {
    Timestamp::new(ClockDomain::Simulation, 0)
}

fn dt() -> Duration {
    Duration::from_millis(20)
}

fn sim_with<C: Controller>(
    params: PlantParams,
    initial: PlantState,
    sensor: DepthSensor,
    controller: C,
) -> Simulation<C> {
    let plant = DepthPlant::new(params, initial).unwrap();
    Simulation::new(start(), dt(), plant, sensor, controller).unwrap()
}

#[test]
fn a_run_is_deterministic() {
    let mut a = sim_with(
        PlantParams::default(),
        PlantState::at_surface(),
        DepthSensor::ideal(),
        ProportionalController::new(5.0, 0.6, 1.0),
    );
    let mut b = sim_with(
        PlantParams::default(),
        PlantState::at_surface(),
        DepthSensor::ideal(),
        ProportionalController::new(5.0, 0.6, 1.0),
    );
    let ta = a.run(400).unwrap();
    let tb = b.run(400).unwrap();
    assert_eq!(ta, tb, "identical runs must produce identical trajectories");
    assert_eq!(ta.len(), 400);
}

#[test]
fn proportional_controller_converges_to_the_setpoint() {
    let mut sim = sim_with(
        PlantParams::default(),
        PlantState::at_surface(),
        DepthSensor::ideal(),
        ProportionalController::new(5.0, 0.6, 1.0),
    );
    let traj = sim.run(1500).unwrap();
    assert!(
        (traj.final_depth() - 5.0).abs() < 0.05,
        "expected convergence near 5 m, got {}",
        traj.final_depth()
    );
}

#[test]
fn constant_thrust_drives_the_vehicle_down() {
    // A fixed downward command with no buoyancy: depth increases monotonically.
    struct Constant(f64);
    impl Controller for Constant {
        fn command(&mut self, _ctx: &StepContext, _measured: f64) -> f64 {
            self.0
        }
    }
    let params = PlantParams {
        buoyancy_accel: 0.0,
        ..PlantParams::default()
    };
    let mut sim = sim_with(
        params,
        PlantState::at_surface(),
        DepthSensor::ideal(),
        Constant(1.0),
    );
    let traj = sim.run(50).unwrap();

    let depths: Vec<f64> = traj.samples.iter().map(|s| s.true_depth).collect();
    for pair in depths.windows(2) {
        assert!(pair[1] >= pair[0], "depth must not decrease: {pair:?}");
    }
    assert!(traj.final_depth() > 0.0);
}

#[test]
fn a_buoyant_vehicle_rises_and_is_clamped_at_the_surface() {
    // No command, positive buoyancy, starting submerged: it rises to the surface
    // and never breaches it.
    struct Idle;
    impl Controller for Idle {
        fn command(&mut self, _ctx: &StepContext, _measured: f64) -> f64 {
            0.0
        }
    }
    let params = PlantParams {
        buoyancy_accel: 0.5,
        ..PlantParams::default()
    };
    let mut sim = sim_with(
        params,
        PlantState::new(3.0, 0.0),
        DepthSensor::ideal(),
        Idle,
    );
    let traj = sim.run(2000).unwrap();

    assert!(traj.samples.iter().all(|s| s.true_depth >= 0.0));
    assert!(
        traj.final_depth() < 0.01,
        "a buoyant idle vehicle should settle at the surface, got {}",
        traj.final_depth()
    );
}

#[test]
fn depth_is_clamped_to_max_depth() {
    struct FullDown;
    impl Controller for FullDown {
        fn command(&mut self, _ctx: &StepContext, _measured: f64) -> f64 {
            1.0
        }
    }
    let params = PlantParams {
        max_depth: 2.0,
        buoyancy_accel: 0.0,
        ..PlantParams::default()
    };
    let mut sim = sim_with(
        params,
        PlantState::at_surface(),
        DepthSensor::ideal(),
        FullDown,
    );
    let traj = sim.run(500).unwrap();
    assert!(traj.samples.iter().all(|s| s.true_depth <= 2.0 + 1e-9));
    assert!((traj.final_depth() - 2.0).abs() < 1e-9);
}

#[test]
fn sensor_bias_and_quantization_shape_the_measurement() {
    let sensor = DepthSensor::new(SensorParams {
        bias: 0.5,
        quantum: 0.25,
    })
    .unwrap();
    // 3.1 m + 0.5 bias = 3.6, rounded to nearest 0.25 -> 3.5.
    assert!((sensor.observe(3.1) - 3.5).abs() < 1e-9);
    // The ideal sensor is the identity.
    assert!((DepthSensor::ideal().observe(3.1) - 3.1).abs() < 1e-9);
}

#[test]
fn invalid_parameters_are_rejected() {
    let bad = DepthPlant::new(
        PlantParams {
            max_depth: 0.0,
            ..PlantParams::default()
        },
        PlantState::at_surface(),
    );
    assert!(matches!(bad, Err(SimError::InvalidParameter { name, .. }) if name == "max_depth"));
}

#[test]
fn a_zero_step_is_rejected() {
    let plant = DepthPlant::new(PlantParams::default(), PlantState::at_surface()).unwrap();
    let err = Simulation::new(
        start(),
        Duration::from_nanos(0),
        plant,
        DepthSensor::ideal(),
        ProportionalController::new(1.0, 0.5, 1.0),
    )
    .unwrap_err();
    assert!(matches!(err, SimError::NonPositiveStep));
}

#[test]
fn timestamps_advance_by_exactly_one_step() {
    let mut sim = sim_with(
        PlantParams::default(),
        PlantState::at_surface(),
        DepthSensor::ideal(),
        ProportionalController::new(5.0, 0.6, 1.0),
    );
    let traj = sim.run(3).unwrap();
    assert_eq!(traj.samples[0].time.as_nanos(), 0);
    assert_eq!(traj.samples[1].time.as_nanos(), 20_000_000);
    assert_eq!(traj.samples[2].time.as_nanos(), 40_000_000);
    // The clock is positioned one step past the last sample.
    assert_eq!(sim.now().as_nanos(), 60_000_000);
}
