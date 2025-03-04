use criterion::{criterion_group, criterion_main, Criterion};
use gemm::f16;
use gemm::*;
use nalgebra::DMatrix;
use std::time::Duration;

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut mnks = vec![];
    let mut push = |m, n, k| {
        mnks.push((m, n, k));
    };
    push(64, 64, 64);
    push(8192, 8192, 8192);
    push(4096, 4096, 4096);
    push(1024, 1024, 1024);
    push(896, 128, 128);
    push(512, 256, 256);
    push(448, 448, 128);
    push(256, 256, 256);
    push(256, 32, 256);
    push(52, 52, 256);
    push(48, 48, 256);
    push(63, 1, 10);
    push(63, 2, 10);
    push(63, 3, 10);
    push(63, 4, 10);

    push(1024, 1, 1024);
    push(1024, 2, 1024);
    push(1024, 3, 1024);
    push(1024, 4, 1024);

    for (m, n, k) in mnks.iter().copied() {
        let a_vec = vec![0.0_f64; m * k];
        let b_vec = vec![0.0_f64; k * n];
        let mut c_vec = vec![0.0_f64; m * n];

        for (dst_label, dst_cs, dst_rs) in [("n", m, 1), ("t", 1, n)] {
            for (lhs_label, lhs_cs, lhs_rs) in [("n", m, 1), ("t", 1, k)] {
                for (rhs_label, rhs_cs, rhs_rs) in [("n", k, 1), ("t", 1, n)] {
                    c.bench_function(
                        &format!(
                            "f64-{}{}{}-gemm-{}×{}×{}",
                            dst_label, lhs_label, rhs_label, m, n, k
                        ),
                        |b| {
                            b.iter(|| unsafe {
                                gemm(
                                    m,
                                    n,
                                    k,
                                    c_vec.as_mut_ptr(),
                                    dst_cs as isize,
                                    dst_rs as isize,
                                    true,
                                    a_vec.as_ptr(),
                                    lhs_cs as isize,
                                    lhs_rs as isize,
                                    b_vec.as_ptr(),
                                    rhs_cs as isize,
                                    rhs_rs as isize,
                                    0.0_f64,
                                    0.0_f64,
                                    false,
                                    false,
                                    false,
                                    gemm::Parallelism::Rayon(0),
                                )
                            })
                        },
                    );
                }
            }
        }

        let a_mat = DMatrix::<f64>::zeros(m, k);
        let b_mat = DMatrix::<f64>::zeros(k, n);
        let mut c_mat = DMatrix::<f64>::zeros(m, n);
        c.bench_function(&format!("f64-nalg-{}×{}×{}", m, n, k), |b| {
            b.iter(|| c_mat = &a_mat * &b_mat)
        });
    }

    for (m, n, k) in mnks.iter().copied() {
        let a_vec = vec![0.0_f32; m * k];
        let b_vec = vec![0.0_f32; k * n];
        let mut c_vec = vec![0.0_f32; m * n];

        for (dst_label, dst_cs, dst_rs) in [("n", m, 1), ("t", 1, n)] {
            for (lhs_label, lhs_cs, lhs_rs) in [("n", m, 1), ("t", 1, k)] {
                for (rhs_label, rhs_cs, rhs_rs) in [("n", k, 1), ("t", 1, n)] {
                    c.bench_function(
                        &format!(
                            "f32-{}{}{}-gemm-{}×{}×{}",
                            dst_label, lhs_label, rhs_label, m, n, k
                        ),
                        |b| {
                            b.iter(|| unsafe {
                                gemm(
                                    m,
                                    n,
                                    k,
                                    c_vec.as_mut_ptr(),
                                    dst_cs as isize,
                                    dst_rs as isize,
                                    true,
                                    a_vec.as_ptr(),
                                    lhs_cs as isize,
                                    lhs_rs as isize,
                                    b_vec.as_ptr(),
                                    rhs_cs as isize,
                                    rhs_rs as isize,
                                    0.0_f32,
                                    0.0_f32,
                                    false,
                                    false,
                                    false,
                                    gemm::Parallelism::Rayon(0),
                                )
                            })
                        },
                    );
                }
            }
        }

        let a_mat = DMatrix::<f32>::zeros(m, k);
        let b_mat = DMatrix::<f32>::zeros(k, n);
        let mut c_mat = DMatrix::<f32>::zeros(m, n);
        c.bench_function(&format!("f32-nalg-{}×{}×{}", m, n, k), |b| {
            b.iter(|| c_mat = &a_mat * &b_mat)
        });
    }

    for (m, n, k) in mnks.iter().copied() {
        let a_vec = vec![f16::ZERO; m * k];
        let b_vec = vec![f16::ZERO; k * n];
        let mut c_vec = vec![f16::ZERO; m * n];

        for (dst_label, dst_cs, dst_rs) in [("n", m, 1), ("t", 1, n)] {
            for (lhs_label, lhs_cs, lhs_rs) in [("n", m, 1), ("t", 1, k)] {
                for (rhs_label, rhs_cs, rhs_rs) in [("n", k, 1), ("t", 1, n)] {
                    c.bench_function(
                        &format!(
                            "f16-{}{}{}-gemm-{}×{}×{}",
                            dst_label, lhs_label, rhs_label, m, n, k
                        ),
                        |b| {
                            b.iter(|| unsafe {
                                gemm(
                                    m,
                                    n,
                                    k,
                                    c_vec.as_mut_ptr(),
                                    dst_cs as isize,
                                    dst_rs as isize,
                                    true,
                                    a_vec.as_ptr(),
                                    lhs_cs as isize,
                                    lhs_rs as isize,
                                    b_vec.as_ptr(),
                                    rhs_cs as isize,
                                    rhs_rs as isize,
                                    f16::ZERO,
                                    f16::ZERO,
                                    false,
                                    false,
                                    false,
                                    gemm::Parallelism::Rayon(0),
                                )
                            })
                        },
                    );

                    c.bench_function(
                        &format!(
                            "f16-{}{}{}-naive-{}×{}×{}",
                            dst_label, lhs_label, rhs_label, m, n, k
                        ),
                        |b| {
                            b.iter(|| unsafe {
                                gemm_fallback(
                                    m,
                                    n,
                                    k,
                                    c_vec.as_mut_ptr(),
                                    dst_cs as isize,
                                    dst_rs as isize,
                                    true,
                                    a_vec.as_ptr(),
                                    lhs_cs as isize,
                                    lhs_rs as isize,
                                    b_vec.as_ptr(),
                                    rhs_cs as isize,
                                    rhs_rs as isize,
                                    f16::ZERO,
                                    f16::ZERO,
                                )
                            })
                        },
                    );
                }
            }
        }

        let a_mat = DMatrix::<f32>::zeros(m, k);
        let b_mat = DMatrix::<f32>::zeros(k, n);
        let mut c_mat = DMatrix::<f32>::zeros(m, n);
        c.bench_function(&format!("f32-nalg-{}×{}×{}", m, n, k), |b| {
            b.iter(|| c_mat = &a_mat * &b_mat)
        });
    }

    for (m, n, k) in mnks.iter().copied() {
        let a_vec = vec![c64::default(); m * k];
        let b_vec = vec![c64::default(); k * n];
        let mut c_vec = vec![c64::default(); m * n];

        for (dst_label, dst_cs, dst_rs) in [("n", m, 1), ("t", 1, n)] {
            for (lhs_label, lhs_cs, lhs_rs) in [("n", m, 1), ("t", 1, k)] {
                for (rhs_label, rhs_cs, rhs_rs) in [("n", k, 1), ("t", 1, n)] {
                    c.bench_function(
                        &format!(
                            "c64-{}{}{}-gemm-{}×{}×{}",
                            dst_label, lhs_label, rhs_label, m, n, k
                        ),
                        |b| {
                            b.iter(|| unsafe {
                                gemm(
                                    m,
                                    n,
                                    k,
                                    c_vec.as_mut_ptr(),
                                    dst_cs as isize,
                                    dst_rs as isize,
                                    true,
                                    a_vec.as_ptr(),
                                    lhs_cs as isize,
                                    lhs_rs as isize,
                                    b_vec.as_ptr(),
                                    rhs_cs as isize,
                                    rhs_rs as isize,
                                    c64::default(),
                                    c64::default(),
                                    false,
                                    false,
                                    false,
                                    gemm::Parallelism::Rayon(0),
                                )
                            })
                        },
                    );
                }
            }
        }
    }
}

pub fn criterion_benchmark_parallelism(c: &mut Criterion) {
    // let mnks = vec![(6, 768 * 3, 768)];
    let mnks = vec![(4096, 128, 11108)];
    // let mut push = |m, n, k| {
    //     mnks.push((m, n, k));
    // };
    // push(64, 64, 64);
    // push(8192, 8192, 8192);
    // push(4096, 4096, 4096);
    // push(1024, 1024, 1024);
    // push(896, 128, 128);
    // push(512, 256, 256);
    // push(448, 448, 128);
    // push(256, 256, 256);
    // push(256, 32, 256);
    // push(52, 52, 256);
    // push(48, 48, 256);
    // push(63, 1, 10);
    // push(63, 2, 10);
    // push(63, 3, 10);
    // push(63, 4, 10);

    // push(1024, 1, 1024);
    // push(1024, 2, 1024);
    // push(1024, 3, 1024);
    // push(1024, 4, 1024);
    //
    let n_cpus = num_cpus::get();

    for (m, n, k) in mnks.iter().copied() {
        let a_vec = vec![0.0_f32; m * k];
        let b_vec = vec![0.0_f32; k * n];
        let mut c_vec = vec![0.0_f32; m * n];

        for (dst_label, dst_cs, dst_rs) in [("n", m, 1), ("t", 1, n)] {
            for (lhs_label, lhs_cs, lhs_rs) in [("n", m, 1), ("t", 1, k)] {
                for (rhs_label, rhs_cs, rhs_rs) in [("n", k, 1), ("t", 1, n)] {
                    c.bench_function(
                        &format!(
                            "parallelism-f32-{}-{}{}{}-gemm-{}×{}×{}",
                            n_cpus, dst_label, lhs_label, rhs_label, m, n, k
                        ),
                        |b| {
                            b.iter(|| unsafe {
                                gemm(
                                    m,
                                    n,
                                    k,
                                    c_vec.as_mut_ptr(),
                                    dst_cs as isize,
                                    dst_rs as isize,
                                    true,
                                    a_vec.as_ptr(),
                                    lhs_cs as isize,
                                    lhs_rs as isize,
                                    b_vec.as_ptr(),
                                    rhs_cs as isize,
                                    rhs_rs as isize,
                                    0.0_f32,
                                    0.0_f32,
                                    false,
                                    false,
                                    false,
                                    gemm::Parallelism::Rayon(n_cpus),
                                )
                            })
                        },
                    );
                    c.bench_function(
                        &format!(
                            "parallelism-f32-none-{}{}{}-gemm-{}×{}×{}",
                            dst_label, lhs_label, rhs_label, m, n, k
                        ),
                        |b| {
                            b.iter(|| unsafe {
                                gemm(
                                    m,
                                    n,
                                    k,
                                    c_vec.as_mut_ptr(),
                                    dst_cs as isize,
                                    dst_rs as isize,
                                    true,
                                    a_vec.as_ptr(),
                                    lhs_cs as isize,
                                    lhs_rs as isize,
                                    b_vec.as_ptr(),
                                    rhs_cs as isize,
                                    rhs_rs as isize,
                                    0.0_f32,
                                    0.0_f32,
                                    false,
                                    false,
                                    false,
                                    gemm::Parallelism::None,
                                )
                            })
                        },
                    );
                }
            }
        }
    }

    let n_cpus = num_cpus::get();

    for (m, n, k) in mnks.iter().copied() {
        let a_vec = vec![f16::from_f32(0.0); m * k];
        let b_vec = vec![f16::from_f32(0.0); k * n];
        let mut c_vec = vec![f16::from_f32(0.0); m * n];

        for (dst_label, dst_cs, dst_rs) in [("n", m, 1), ("t", 1, n)] {
            for (lhs_label, lhs_cs, lhs_rs) in [("n", m, 1), ("t", 1, k)] {
                for (rhs_label, rhs_cs, rhs_rs) in [("n", k, 1), ("t", 1, n)] {
                    c.bench_function(
                        &format!(
                            "parallelism-f16-{}-{}{}{}-gemm-{}×{}×{}",
                            n_cpus, dst_label, lhs_label, rhs_label, m, n, k
                        ),
                        |b| {
                            b.iter(|| unsafe {
                                gemm(
                                    m,
                                    n,
                                    k,
                                    c_vec.as_mut_ptr(),
                                    dst_cs as isize,
                                    dst_rs as isize,
                                    true,
                                    a_vec.as_ptr(),
                                    lhs_cs as isize,
                                    lhs_rs as isize,
                                    b_vec.as_ptr(),
                                    rhs_cs as isize,
                                    rhs_rs as isize,
                                    f16::from_f32(0.0),
                                    f16::from_f32(0.0),
                                    false,
                                    false,
                                    false,
                                    gemm::Parallelism::Rayon(n_cpus),
                                )
                            })
                        },
                    );
                    c.bench_function(
                        &format!(
                            "parallelism-f16-none-{}{}{}-gemm-{}×{}×{}",
                            dst_label, lhs_label, rhs_label, m, n, k
                        ),
                        |b| {
                            b.iter(|| unsafe {
                                gemm(
                                    m,
                                    n,
                                    k,
                                    c_vec.as_mut_ptr(),
                                    dst_cs as isize,
                                    dst_rs as isize,
                                    true,
                                    a_vec.as_ptr(),
                                    lhs_cs as isize,
                                    lhs_rs as isize,
                                    b_vec.as_ptr(),
                                    rhs_cs as isize,
                                    rhs_rs as isize,
                                    f16::from_f32(0.0),
                                    f16::from_f32(0.0),
                                    false,
                                    false,
                                    false,
                                    gemm::Parallelism::None,
                                )
                            })
                        },
                    );
                }
            }
        }
    }
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(2))
        .sample_size(10);
    targets = criterion_benchmark
);
criterion_group!(
    name = benches_parallelism;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(2))
        .sample_size(10);
    targets = criterion_benchmark_parallelism
);
criterion_main!(benches, benches_parallelism);
