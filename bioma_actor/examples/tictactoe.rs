use bioma_actor::prelude::*;
use futures::StreamExt;
use rand::prelude::*;
use serde::{Deserialize, Serialize};
use std::future::Future;
use tracing::{error, info};

use rerun::external::glam;

const BOARD_POSITIONS: [glam::Vec3; 9] = [
    glam::Vec3::new(0.0, 0.0, 0.0),
    glam::Vec3::new(1.0, 0.0, 0.0),
    glam::Vec3::new(2.0, 0.0, 0.0),
    glam::Vec3::new(0.0, 0.0, -1.0),
    glam::Vec3::new(1.0, 0.0, -1.0),
    glam::Vec3::new(2.0, 0.0, -1.0),
    glam::Vec3::new(0.0, 0.0, -2.0),
    glam::Vec3::new(1.0, 0.0, -2.0),
    glam::Vec3::new(2.0, 0.0, -2.0),
];

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
enum PlayerType {
    X,
    O,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StartGame;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MakeMove {
    player: PlayerType,
    position: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GameState {
    board: [Option<PlayerType>; 9],
    current_player: PlayerType,
    game_over: bool,
    winner: Option<PlayerType>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GameResult {
    board: [Option<PlayerType>; 9],
    winner: Option<PlayerType>,
}

// Game actor
#[derive(Clone, Debug, Serialize, Deserialize)]
struct GameActor {
    player_x: ActorId,
    player_o: ActorId,
    board: ActorId,
    current_player: PlayerType,
    rerun_id: ActorId,
}

impl Message<StartGame> for GameActor {
    type Response = ();

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        _: &StartGame,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        let current_player = if self.current_player == PlayerType::X { &self.player_x } else { &self.player_o };
        let rerun_id = &self.rerun_id;
        let game_state =
            GameState { board: [None; 9], current_player: self.current_player, game_over: false, winner: None };
        info!("{}: Sending GameState to player: {:?}", ctx.id(), self.current_player);
        async move {
            ctx.do_send::<PlayerActor, GameState>(game_state.clone(), current_player).await?;
            ctx.do_send::<RerunActor, GameState>(game_state.clone(), rerun_id).await?;
            Ok(())
        }
    }
}

impl Message<GameResult> for GameActor {
    type Response = ();

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        result: &GameResult,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        async move {
            info!("{} {:?}", ctx.id(), result);
            ctx.do_send::<PlayerActor, GameResult>(result.clone(), &self.player_x).await?;
            ctx.do_send::<PlayerActor, GameResult>(result.clone(), &self.player_o).await?;
            Ok(())
        }
    }
}

impl Actor for GameActor {
    type Error = SystemActorError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            info!("{} Started", ctx.id());

            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(game_result) = frame.is::<GameResult>() {
                    self.reply(ctx, &game_result, &frame).await?;
                    break;
                } else if let Some(start_game) = frame.is::<StartGame>() {
                    self.reply(ctx, &start_game, &frame).await?;
                }
            }
            info!("{} Finished", ctx.id());
            Ok(())
        }
    }
}

// Player actor
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PlayerActor {
    player_type: PlayerType,
    game: ActorId,
    board: ActorId,
    rerun_id: ActorId,
}

impl Message<GameState> for PlayerActor {
    type Response = ();

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        state: &GameState,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        let player_type = self.player_type;
        let board = self.board.clone();
        let state = state.clone();
        async move {
            info!("{} Analyzing GameState", ctx.id());
            if state.current_player == player_type {
                let empty_positions: Vec<usize> = state
                    .board
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &cell)| if cell.is_none() { Some(i) } else { None })
                    .collect();

                if !empty_positions.is_empty() {
                    let mut rng = StdRng::from_entropy();
                    let random_position = empty_positions[rng.gen_range(0..empty_positions.len())];
                    info!("{} Making move at position {}", ctx.id(), random_position);
                    ctx.do_send::<RerunActor, MakeMove>(
                        MakeMove { player: player_type, position: random_position },
                        &self.rerun_id,
                    ).await?;
                    ctx.do_send::<BoardActor, MakeMove>(
                        MakeMove { player: player_type, position: random_position },
                        &board,
                    )
                    .await?;
                } else {
                    info!("{} No empty positions available", ctx.id());
                }
            }
            Ok(())
        }
    }
}

impl Message<GameResult> for PlayerActor {
    type Response = ();

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        result: &GameResult,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        async move {
            match result.winner {
                Some(winner) if winner == self.player_type => info!("{} Player {:?} wins!", ctx.id(), self.player_type),
                Some(_) => info!("{} Player {:?} loses!", ctx.id(), self.player_type),
                None => info!("{} It's a draw!", ctx.id()),
            }
            Ok(())
        }
    }
}

impl Actor for PlayerActor {
    type Error = SystemActorError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            info!("{} {:?} started", ctx.id(), self.player_type);
            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(game_state) = frame.is::<GameState>() {
                    self.reply(ctx, &game_state, &frame).await?;
                } else if let Some(game_result) = frame.is::<GameResult>() {
                    self.reply(ctx, &game_result, &frame).await?;
                    break;
                }
            }
            info!("{} {:?} finished", ctx.id(), self.player_type);
            Ok(())
        }
    }
}

// Board actor
#[derive(Clone, Debug, Serialize, Deserialize)]
struct BoardActor {
    game: ActorId,
    player_x: ActorId,
    player_o: ActorId,
    rerun_id: ActorId,
    board: [Option<PlayerType>; 9],
    current_player: PlayerType,
    game_over: bool,
}

impl BoardActor {
    fn check_winner(&self) -> Option<PlayerType> {
        const WINNING_COMBINATIONS: [[usize; 3]; 8] = [
            [0, 1, 2],
            [3, 4, 5],
            [6, 7, 8], // Rows
            [0, 3, 6],
            [1, 4, 7],
            [2, 5, 8], // Columns
            [0, 4, 8],
            [2, 4, 6], // Diagonals
        ];

        for combo in &WINNING_COMBINATIONS {
            if let (Some(player), true) = (
                self.board[combo[0]],
                self.board[combo[0]] == self.board[combo[1]] && self.board[combo[1]] == self.board[combo[2]],
            ) {
                return Some(player);
            }
        }

        None
    }

    fn is_full(&self) -> bool {
        self.board.iter().all(|cell| cell.is_some())
    }

    fn draw_board(&self) -> String {
        let mut board_str = String::new();
        for i in 0..3 {
            for j in 0..3 {
                let index = i * 3 + j;
                let symbol = match self.board[index] {
                    Some(PlayerType::X) => "X",
                    Some(PlayerType::O) => "O",
                    None => " ",
                };
                board_str.push_str(symbol);
                if j < 2 {
                    board_str.push('|');
                }
            }
            board_str.push('\n');
            if i < 2 {
                board_str.push_str("-+-+-\n");
            }
        }
        board_str
    }
}

impl Message<MakeMove> for BoardActor {
    type Response = ();

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        move_msg: &MakeMove,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        async move {
            info!("{} Received MakeMove: player {:?}, position {}", ctx.id(), move_msg.player, move_msg.position);

            // Check if the move is valid
            if move_msg.player == self.current_player && self.board[move_msg.position].is_none() {
                info!("{} Move is valid. Updating board state.", ctx.id());
                self.board[move_msg.position] = Some(move_msg.player);

                info!("{} Current board state:\n{}", ctx.id(), self.draw_board());

                let winner = self.check_winner();
                info!("{} Winner: {:?}", ctx.id(), winner);

                // Send to Rerun actor
                ctx.do_send::<RerunActor, GameState>(
                    GameState {
                        board: self.board,
                        current_player: self.current_player,
                        game_over: self.game_over,
                        winner,
                    },
                    &self.rerun_id,
                )
                .await?;

                let is_full = self.is_full();
                info!("{} Board full: {}", ctx.id(), is_full);

                self.game_over = winner.is_some() || is_full;
                info!("{} Game over: {}", ctx.id(), self.game_over);

                if self.game_over {
                    info!("{} Game is over. Sending GameResult to {}", ctx.id(), &self.game);
                    ctx.do_send::<GameActor, GameResult>(GameResult { board: self.board, winner }, &self.game).await?;
                    ctx.do_send::<RerunActor, GameResult>(GameResult { board: self.board, winner }, &self.rerun_id).await?;
                } else {
                    info!("{} Game continues. Switching current player.", ctx.id());
                    self.current_player = match self.current_player {
                        PlayerType::X => PlayerType::O,
                        PlayerType::O => PlayerType::X,
                    };

                    info!("{} Current player is now: {:?}", ctx.id(), self.current_player);

                    let next_player =
                        if self.current_player == PlayerType::X { &self.player_x } else { &self.player_o };

                    info!("{} Sending updated GameState to next player: {:?}", ctx.id(), self.current_player);
                    ctx.do_send::<PlayerActor, GameState>(
                        GameState {
                            board: self.board,
                            current_player: self.current_player,
                            game_over: self.game_over,
                            winner,
                        },
                        next_player,
                    )
                    .await?;
                    ctx.do_send::<PlayerActor, GameState>(
                        GameState {
                            board: self.board,
                            current_player: self.current_player,
                            game_over: self.game_over,
                            winner,
                        },
                        &self.rerun_id,
                    )
                    .await?;
                }
            } else {
                if move_msg.player != self.current_player {
                    error!(
                        "{} Invalid move: Not the current player's turn. Current player: {:?}, Move attempt by: {:?}",
                        ctx.id(),
                        self.current_player,
                        move_msg.player
                    );
                } else {
                    let player_at_position = self.board[move_msg.position];
                    error!(
                        "{} Invalid move: Position {} is already occupied by {:?}",
                        ctx.id(),
                        move_msg.position,
                        player_at_position
                    );
                }
            }
            Ok(())
        }
    }
}

impl Actor for BoardActor {
    type Error = SystemActorError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            info!("{} Started", ctx.id());

            // Draw the initial empty board
            info!("{} Initial board state:\n{}", ctx.id(), self.draw_board());

            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(move_msg) = frame.is::<MakeMove>() {
                    self.reply(ctx, &move_msg, &frame).await?;
                }
                if self.game_over {
                    break;
                }
            }
            info!("{} Finished", ctx.id());
            Ok(())
        }
    }
}

// Main actor
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MainActor;

impl Actor for MainActor {
    type Error = SystemActorError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            info!("{} Started", ctx.id());
            // Create actor IDs
            let game_id = ActorId::of::<GameActor>("/game");
            let board_id = ActorId::of::<BoardActor>("/board");
            let player_x_id = ActorId::of::<PlayerActor>("/player_x");
            let player_o_id = ActorId::of::<PlayerActor>("/player_o");
            let rerun_id = ActorId::of::<RerunActor>("/rerun");

            // Spawn game actor
            let mut game_actor = Actor::spawn(
                &ctx.engine(),
                &game_id,
                GameActor {
                    player_x: player_x_id.clone(),
                    player_o: player_o_id.clone(),
                    board: board_id.clone(),
                    current_player: PlayerType::X,
                    rerun_id: rerun_id.clone(),
                },
            )
            .await?;

            // Spawn board actor
            let mut board_actor = Actor::spawn(
                &ctx.engine(),
                &board_id,
                BoardActor {
                    game: game_id.clone(),
                    player_x: player_x_id.clone(),
                    player_o: player_o_id.clone(),
                    rerun_id: rerun_id.clone(),
                    board: [None; 9],
                    current_player: PlayerType::X,
                    game_over: false,
                },
            )
            .await?;

            // Spawn player_x actor
            let mut player_x_actor = Actor::spawn(
                &ctx.engine(),
                &player_x_id,
                PlayerActor {
                    player_type: PlayerType::X,
                    game: game_id.clone(),
                    board: board_id.clone(),
                    rerun_id: rerun_id.clone(),
                },
            )
            .await?;

            // Spawn player_o actor
            let mut player_o_actor = Actor::spawn(
                &ctx.engine(),
                &player_o_id,
                PlayerActor {
                    player_type: PlayerType::O,
                    game: game_id.clone(),
                    board: board_id.clone(),
                    rerun_id: rerun_id.clone(),
                },
            )
            .await?;

            // Spawn rerun actor
            let mut rerun_actor = Actor::spawn(
                &ctx.engine(),
                &rerun_id,
                RerunActor { rec: None, board: [None; 9], data_points: vec![], colors: vec![] },
            )
            .await?;

            // Start the game actor
            let game_handle = tokio::spawn(async move {
                game_actor.start().await.unwrap();
            });

            // Start the board actor
            let board_handle = tokio::spawn(async move {
                board_actor.start().await.unwrap();
            });

            // Start the player_x actor
            let player_x_handle = tokio::spawn(async move {
                player_x_actor.start().await.unwrap();
            });

            // Start the player_o actor
            let player_o_handle = tokio::spawn(async move {
                player_o_actor.start().await.unwrap();
            });

            // Start the rerun actor
            let rerun_handle = tokio::spawn(async move {
                rerun_actor.start().await.unwrap();
            });

            // Start the game actor
            tokio::time::sleep(std::time::Duration::from_secs(0)).await;
            ctx.do_send::<GameActor, StartGame>(StartGame, &game_id).await?;

            info!("{} Started game", ctx.id());

            // Wait for all actors to finish
            let _ = game_handle.await;
            let _ = board_handle.await;
            let _ = player_x_handle.await;
            let _ = player_o_handle.await;
            let _ = rerun_handle.await;

            info!("{} Finished", ctx.id());
            Ok(())
        }
    }
}

// Rerun actor
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RerunActor {
    #[serde(skip)]
    rec: Option<rerun::RecordingStream>,
    #[serde(skip)]
    board: [Option<PlayerType>; 9],
    #[serde(skip)]
    data_points: Vec<glam::Vec3>,
    #[serde(skip)]
    colors: Vec<rerun::Color>,
}

impl RerunActor {
    fn show_current_player(&self) -> Result<(), Box<dyn std::error::Error>> {
        let current_player = match self.board.iter().filter(|&cell| cell.is_some()).count() % 2 {
            0 => PlayerType::X,
            _ => PlayerType::O,
        };
        let color = match current_player {
            PlayerType::X => rerun::Color::from_rgb(255, 0, 0),
            PlayerType::O => rerun::Color::from_rgb(0, 0, 255),
        };
        let position = glam::Vec3::new(-1.0, 0.0, 1.0);
        self.rec
            .as_ref()
            .unwrap()
            .log("CurrentPlayer", &rerun::Points3D::new(vec![position]).with_colors(vec![color]).with_radii([0.3]))?;
        Ok(())
    }

    fn update_board(&mut self, board: [Option<PlayerType>; 9]) {
        self.board = board;
    }

    fn draw_board(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Draw grid
        let mut boxes = vec![];
        for (idx, player_type) in self.board.iter().enumerate() {
            if let Some(player) = player_type {
                let color = match player {
                    PlayerType::X => rerun::Color::from_rgb(255, 0, 0),
                    PlayerType::O => rerun::Color::from_rgb(0, 0, 255),
                };
                self.data_points.push(BOARD_POSITIONS[idx]);
                self.colors.push(color);
            }
            boxes.push(BOARD_POSITIONS[idx]);
        }

        self.rec.as_ref().unwrap().log(
            "Grid",
            &rerun::Boxes3D::from_centers_and_half_sizes(boxes, [glam::Vec3::new(0.5, 0.5, 0.5); 9])
                .with_colors(vec![rerun::Color::from_rgb(255, 255, 255); 9]),
        )?;

        self.rec.as_ref().unwrap().log(
            "Datapoints",
            &rerun::Points3D::new(self.data_points.clone()).with_colors(self.colors.clone()).with_radii([0.3]),
        )?;

        Ok(())
    }

    fn draw_winner(&self, winner: Option<PlayerType>) -> Result<(), Box<dyn std::error::Error>> {
        let color = match winner {
            Some(PlayerType::X) => rerun::Color::from_rgb(255, 0, 0),
            Some(PlayerType::O) => rerun::Color::from_rgb(0, 0, 255),
            None => rerun::Color::from_rgb(0, 0, 0),
        };
        let position = glam::Vec3::new(-1.0, 0.0, 1.0);
        self.rec.as_ref().unwrap().log(
            "CurrentPlayer",
            &rerun::Points3D::new(vec![position])
                .with_colors(vec![color])
                .with_radii([0.3])
                .with_labels(vec!["Winner"]),
        )?;
        Ok(())
    }
}

// Impl all message types for rerun actor
impl Message<MakeMove> for RerunActor {
    type Response = ();

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        move_msg: &MakeMove,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        async move {
            info!("{} {:?}", ctx.id(), move_msg);
            let _ = self.show_current_player();
            Ok(())
        }
    }
}
impl Message<GameState> for RerunActor {
    type Response = ();

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        state: &GameState,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        async move {
            info!("{} {:?}", ctx.id(), state);
            let _ = self.update_board(state.board);
            let _ = self.draw_board();
            Ok(())
        }
    }
}

impl Message<GameResult> for RerunActor {
    type Response = ();

    fn handle(
        &mut self,
        ctx: &mut ActorContext<Self>,
        result: &GameResult,
    ) -> impl Future<Output = Result<Self::Response, Self::Error>> {
        async move {
            info!("{} {:?}", ctx.id(), result);
            let _ = self.update_board(result.board);
            let _ = self.draw_board();
            let _ = self.draw_winner(result.winner);
            Ok(())
        }
    }
}

impl Actor for RerunActor {
    type Error = SystemActorError;

    fn start(&mut self, ctx: &mut ActorContext<Self>) -> impl Future<Output = Result<(), Self::Error>> {
        async move {
            info!("{} Started", ctx.id());
            let rec_res = rerun::RecordingStreamBuilder::new("tictactoe.rerun").connect();
            if let Ok(rec) = rec_res {
                self.rec = Some(rec);
            } else {
                self.rec = None
            }
            // Draw initial board
            let _ = self.draw_board();

            let mut stream = ctx.recv().await?;
            while let Some(Ok(frame)) = stream.next().await {
                if let Some(game_state) = frame.is::<GameState>() {
                    self.reply(ctx, &game_state, &frame).await?;
                } else if let Some(game_result) = frame.is::<GameResult>() {
                    self.reply(ctx, &game_result, &frame).await?;
                    break;
                } else if let Some(make_move) = frame.is::<MakeMove>() {
                    self.reply(ctx, &make_move, &frame).await?;
                }
            }
            info!("{} Finished", ctx.id());
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), SystemActorError> {
    let filter = match tracing_subscriber::EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => tracing_subscriber::EnvFilter::new("info"),
    };

    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

    color_backtrace::install();
    color_backtrace::BacktracePrinter::new().message("BOOM! 💥").install(color_backtrace::default_output_stream());

    let engine = Engine::test().await?;

    // Setup the main actor
    let mut main_actor = Actor::spawn(&engine, &ActorId::of::<MainActor>("/main"), MainActor).await?;

    // Start the main actor
    main_actor.start().await?;

    // Export the database for debugging
    dbg_export_db!(engine);

    Ok(())
}
