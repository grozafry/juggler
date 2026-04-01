package main

import (
	"database/sql"
	"log"
	"os"
	"time"

	"github.com/ClickHouse/clickhouse-go/v2"
	_ "github.com/lib/pq"
	"github.com/gin-gonic/gin"
	"juggler/admin-api/internal/handlers"
)

func main() {
	// ── ClickHouse ──────────────────────────────────────────────────────────────
	var ch *sql.DB

	chAddr := os.Getenv("CLICKHOUSE_ADDR")
	if chAddr == "" {
		chAddr = "127.0.0.1:9000"
	}
	chUser := os.Getenv("CLICKHOUSE_USER")
	if chUser == "" {
		chUser = "juggler"
	}
	chPass := os.Getenv("CLICKHOUSE_PASSWORD")
	if chPass == "" {
		chPass = "password"
	}

	for i := 0; i < 10; i++ {
		db := clickhouse.OpenDB(&clickhouse.Options{
			Addr: []string{chAddr},
			Auth: clickhouse.Auth{
				Database: "default",
				Username: chUser,
				Password: chPass,
			},
		})
		if db.Ping() == nil {
			ch = db
			break
		}
		log.Printf("Waiting for ClickHouse... (%d/10)", i+1)
		time.Sleep(2 * time.Second)
	}
	if ch == nil {
		log.Fatal("Failed to connect to ClickHouse after 10 retries")
	}
	defer ch.Close()
	log.Println("✓ Connected to ClickHouse")

	// ── Postgres ────────────────────────────────────────────────────────────────
	pgURL := os.Getenv("DATABASE_URL")
	if pgURL == "" {
		pgURL = "postgres://juggler:password@localhost:5433/gateway_auth?sslmode=disable"
	}
	var pg *sql.DB
	var pgErr error
	for i := 0; i < 5; i++ {
		pg, pgErr = sql.Open("postgres", pgURL)
		if pgErr == nil && pg.Ping() == nil {
			break
		}
		log.Printf("Waiting for Postgres... (%d/5)", i+1)
		time.Sleep(2 * time.Second)
	}
	if pgErr != nil || pg.Ping() != nil {
		log.Println("⚠️  Postgres not available — Virtual Keys API will be disabled")
		pg = nil
	} else {
		log.Println("✓ Connected to Postgres")
	}

	// ── Auth middleware ──────────────────────────────────────────────────────────
	adminKey := os.Getenv("ADMIN_API_KEY")
	authMiddleware := func(c *gin.Context) {
		if adminKey == "" || adminKey == "changeme" {
			// No key set — open access (dev mode)
			c.Next()
			return
		}
		key := c.GetHeader("X-Admin-Key")
		if key == "" {
			key = c.Query("admin_key")
		}
		if key != adminKey {
			c.AbortWithStatusJSON(401, gin.H{"error": "unauthorized"})
			return
		}
		c.Next()
	}

	// ── Router ──────────────────────────────────────────────────────────────────
	gin.SetMode(gin.ReleaseMode)
	router := gin.New()
	router.Use(gin.Recovery())
	router.Use(func(c *gin.Context) {
		c.Header("Access-Control-Allow-Origin", "*")
		c.Header("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS")
		c.Header("Access-Control-Allow-Headers", "Content-Type")
		if c.Request.Method == "OPTIONS" {
			c.AbortWithStatus(204)
			return
		}
		c.Next()
	})

	// Dashboard static file
	router.StaticFile("/", "./dashboard.html")

	analyticsHandler := handlers.NewAnalyticsHandler(ch)
	latencyHandler   := handlers.NewLatencyHandler(ch)

	v1 := router.Group("/admin/v1")
	v1.Use(authMiddleware)
	{
		// Analytics
		v1.GET("/analytics/summary",  analyticsHandler.GetSummary)
		v1.GET("/analytics/timeline", analyticsHandler.GetTimeline)
		v1.GET("/analytics/providers",analyticsHandler.GetProviders)
		v1.GET("/analytics/models",   analyticsHandler.GetModels)
		v1.GET("/analytics/recent",   analyticsHandler.GetRecentLogs)
		v1.GET("/analytics/costs",    analyticsHandler.GetCosts)
		v1.GET("/analytics/latency",  latencyHandler.GetLatency)
		v1.GET("/analytics/errors",   latencyHandler.GetErrors)

		// Virtual Keys (Postgres-backed)
		if pg != nil {
			keysHandler := handlers.NewKeysHandler(pg)
			v1.GET("/keys",        keysHandler.ListKeys)
			v1.POST("/keys",       keysHandler.CreateKey)
			v1.DELETE("/keys/:id", keysHandler.RevokeKey)
		}
	}

	log.Println("✓ Juggler Admin Dashboard → http://localhost:8081")
	if err := router.Run(":8081"); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}
