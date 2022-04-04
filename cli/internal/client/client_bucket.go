package client

import (
	"bytes"
	"context"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/ioutil"
	"net/url"
	"strconv"
	"strings"
	"sync/atomic"
	"time"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/aws/retry"
	awshttp "github.com/aws/aws-sdk-go-v2/aws/transport/http"

	awsconfig "github.com/aws/aws-sdk-go-v2/config"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"github.com/aws/aws-sdk-go-v2/service/s3/types"
	"github.com/aws/smithy-go"
	awslog "github.com/aws/smithy-go/logging"
	hclog "github.com/hashicorp/go-hclog"
)

type BucketClient struct {
	config *ClientConfig
	// Number of failed requests before we stop trying to upload/download artifacts to the remote cache
	maxRemoteFailCount uint64
	// Must be used via atomic package
	currentFailCount uint64
	awsConfig        aws.Config
	configured       bool
	ServiceClient    s3.Client
}

func newBucketClient(config *ClientConfig) (*BucketClient, error) {
	client := &BucketClient{
		config:             config,
		maxRemoteFailCount: config.MaxRemoteFailCount,
	}
	awsConfig, awsConfigErr := client._getAwsSdkConfig()
	client.awsConfig = awsConfig
	client.configured = awsConfigErr == nil
	if awsConfigErr != nil {
		return client, fmt.Errorf("could not configure AWS SDK: %w", awsConfigErr)
	}
	// setting o.UsePathStyle here requires endpoint to have HostnameImmutable=true,
	// otherwise, will have to move the option function to each ServiceClient.requestFunction
	client.ServiceClient = *s3.NewFromConfig(awsConfig, func(o *s3.Options) {
		o.UsePathStyle = config.BucketPathStyle
	})
	// AWS SDK does not validate everything, validate and test manually
	if awsConfigErr = client._verifyAwsSdkConfig(&awsConfig, &client.ServiceClient); awsConfigErr != nil {
		return client, fmt.Errorf("could not configure AWS SDK: %w", awsConfigErr)
	}
	return client, nil
}

func (c *BucketClient) _verifyAwsSdkConfig(config *aws.Config, client *s3.Client) error {
	if config.Region == "" {
		return fmt.Errorf("bucket region is required")
	}
	endpoint, err := config.EndpointResolverWithOptions.ResolveEndpoint(s3.ServiceID, config.Region)
	if err != nil {
		return err
	}
	if endpoint.URL == "" {
		return fmt.Errorf("api url is required")
	}
	if !strings.HasPrefix(endpoint.URL, "http://") && !strings.HasPrefix(endpoint.URL, "https://") {
		return fmt.Errorf("api url requires a scheme")
	}
	if c.config.BucketName == "" {
		return fmt.Errorf("bucket name is required")
	}
	creds, err := config.Credentials.Retrieve(context.TODO())
	if err != nil {
		return err
	}
	println(creds.AccessKeyID, creds.SecretAccessKey, creds.SessionToken)
	if (creds.AccessKeyID == "" || creds.SecretAccessKey == "") && creds.SessionToken == "" {
		return fmt.Errorf("access key id and secret access key are required")
	}
	if creds.Expired() {
		return fmt.Errorf("credentials have expired")
	}
	// FIXME: verify can connect to instance
	// FIXME: verify can read and write to instance
	return nil
}

func (c *BucketClient) _getEndpointResolver() aws.EndpointResolverWithOptions {
	return aws.EndpointResolverWithOptionsFunc(func(service, region string, options ...interface{}) (aws.Endpoint, error) {
		return aws.Endpoint{
			PartitionID:   c.config.BucketPartition,
			URL:           c.config.ApiUrl,
			SigningRegion: c.config.BucketRegion,
			// BucketPathStyle=true means sdk must not modify hostname
			HostnameImmutable: c.config.BucketPathStyle,
		}, nil
	})
}

func (c *BucketClient) _retryCachePolicy(err error) aws.Ternary {
	if errors.As(err, &x509.UnknownAuthorityError{}) {
		// Don't retry if the error was due to TLS cert verification failure.
		atomic.AddUint64(&c.currentFailCount, 1)
		return aws.FalseTernary
	}

	var apiError smithy.APIError
	if errors.As(err, &apiError) {
		// FIXME: remove debug statements or put them behind debug flag
		println(fmt.Sprintf("retry cache api %v", err))
		return aws.FalseTernary
	}

	var httpResponseError *awshttp.ResponseError
	if errors.As(err, &httpResponseError) {
		println(fmt.Sprintf("retry cache http %v", err))
		code := httpResponseError.HTTPStatusCode()
		// Check the response code. We retry on 500-range responses to allow
		// the server time to recover, as 500's are typically not permanent
		// errors and may relate to outages on the server side.
		if code >= 500 && code != 501 && code != 503 {
			atomic.AddUint64(&c.currentFailCount, 1)
			return aws.TrueTernary
		}
	}

	var s3ResponseError s3.ResponseError
	if errors.As(err, &s3ResponseError) {
		println(fmt.Sprintf("retry cache s3 %v", err))
		return aws.FalseTernary
	}

	println(fmt.Sprintf("retry cache swallow %v", err))
	// swallow the error and stop retrying
	return aws.FalseTernary
}

func (c *BucketClient) _checkRetry(err error) aws.Ternary {
	shouldRetry := c._retryCachePolicy(err)
	if shouldRetry.Bool() {
		if retryErr := c.okToRequest(); retryErr != nil {
			return aws.FalseTernary
		}
	}
	return shouldRetry
}

// okToRequest returns nil if it's ok to make a request, and returns the error to
// return to the caller if a request is not allowed
func (c *BucketClient) okToRequest() error {
	if atomic.LoadUint64(&c.currentFailCount) < c.maxRemoteFailCount {
		return nil
	}
	return ErrTooManyFailures
}

func (c *BucketClient) _getRetryer() func() aws.Retryer {
	return func() aws.Retryer {
		return retry.NewStandard(func(so *retry.StandardOptions) {
			// prepend custom retryable, otherwise does nothing
			var original []retry.IsErrorRetryable
			original = append(original, so.Retryables...)
			so.Retryables = []retry.IsErrorRetryable{
				retry.IsErrorRetryableFunc(c._checkRetry),
			}
			so.Retryables = append(so.Retryables, original...)
			so.MaxAttempts = 2
			so.MaxBackoff = 10 * time.Second
		})
	}
}

type AWSLogger struct {
	log hclog.Logger
}

func (a *AWSLogger) Logf(classification awslog.Classification, s string, v ...interface{}) {
	if classification == awslog.Warn {
		a.log.Warn(fmt.Sprintf(s, v...))
	} else {
		a.log.Info(fmt.Sprintf(s, v...))
	}
}

func (c *BucketClient) _getAwsSdkConfig() (aws.Config, error) {
	configOptions := []func(*awsconfig.LoadOptions) error{
		awsconfig.WithHTTPClient(awshttp.NewBuildableClient().WithTimeout(time.Duration(20 * time.Second))),
		awsconfig.WithRetryer(c._getRetryer()),
		awsconfig.WithLogger(&AWSLogger{log: c.config.Logger}),
		awsconfig.WithClientLogMode(aws.LogRequest | aws.LogRetries),
		awsconfig.WithRegion(c.config.BucketRegion),
		awsconfig.WithEndpointResolverWithOptions(c._getEndpointResolver()),
	}
	if c.config.AccessKeyId != "" && c.config.SecretAccessKey != "" {
		// FIXME: determine if session token will break anything
		configOptions = append(configOptions,
			awsconfig.WithCredentialsProvider(credentials.NewStaticCredentialsProvider(c.config.AccessKeyId, c.config.SecretAccessKey, c.config.Token)))
	}
	return awsconfig.LoadDefaultConfig(
		context.TODO(),
		configOptions...,
	)
}

func (c *BucketClient) IsLoggedIn() bool {
	return c.configured
}

// reloads AWS SDK configuration which may cause c.configured to change
func (c *BucketClient) SetToken(token string) {
	c.config.Token = token
	awsConfig, awsConfigErr := c._getAwsSdkConfig()
	c.awsConfig = awsConfig
	c.configured = awsConfigErr == nil
	c.ServiceClient = *s3.NewFromConfig(awsConfig)
}

type LenReader interface {
	Len() int
}

// adapted from go-retryablehttp/client.go/getBodyReaderAndContentLength
func getBodyReaderAndContentLength(rawBody interface{}) (io.Reader, int64, error) {
	var bodyReader io.Reader
	var contentLength int64

	switch body := rawBody.(type) {
	case func() (io.Reader, error):
		// function given, call it to get Reader
		bodyReader, err := body()
		if err != nil {
			return nil, 0, err
		}
		if lr, ok := bodyReader.(LenReader); ok {
			contentLength = int64(lr.Len())
		}

	case []byte:
		// regular byte slice, can use new readers to enable reuse
		buf := body
		bodyReader = bytes.NewReader(buf)
		contentLength = int64(len(buf))

	case *bytes.Buffer:
		// bytes.Buffer, can be reused
		buf := body
		bodyReader = bytes.NewReader(buf.Bytes())
		contentLength = int64(buf.Len())

	case *bytes.Reader:
		// prioritize *bytes.Reader to avoid seek (io.ReadSeeker case)
		buf, err := ioutil.ReadAll(body)
		if err != nil {
			return nil, 0, err
		}
		bodyReader = bytes.NewReader(buf)
		contentLength = int64(len(buf))

	case io.ReadSeeker:
		// compatibility case
		_, err := body.Seek(0, 0)
		if err != nil {
			return nil, 0, err
		}
		bodyReader = ioutil.NopCloser(body)
		if lr, ok := body.(LenReader); ok {
			contentLength = int64(lr.Len())
		}

	case io.Reader:
		// read all in then reset
		buf, err := ioutil.ReadAll(body)
		if err != nil {
			return nil, 0, err
		}
		bodyReader = bytes.NewReader(buf)
		contentLength = int64(len(buf))

	case nil:
		// no body given, nothing to do

	default:
		// unrecognized type
		return nil, 0, fmt.Errorf("cannot handle type %T", rawBody)
	}
	return bodyReader, contentLength, nil
}

func (c *BucketClient) PutArtifact(hash string, artifactBody interface{}, duration int, tag string) error {
	println(fmt.Sprintf("!!!!!!!!! put artifact %v", hash))
	if err := c.okToRequest(); err != nil {
		return err
	}
	params := url.Values{}
	c.addTeamParam(&params)
	// only add a ? if it's actually needed (makes logging cleaner)
	encoded := params.Encode()
	if encoded != "" {
		encoded = "?" + encoded
	}
	bodyReader, contentLength, err := getBodyReaderAndContentLength(artifactBody)
	if err != nil {
		return fmt.Errorf("cannot store artifact: %w", err)
	}
	_, err = c.ServiceClient.PutObject(context.TODO(), &s3.PutObjectInput{
		Bucket:       aws.String(c.config.BucketName),
		Key:          aws.String(c.config.BucketPrefix + "/v8/artifacts/" + hash + encoded),
		RequestPayer: types.RequestPayerRequester,
		Metadata: map[string]string{
			"artifact-duration": fmt.Sprintf("%v", duration),
			"artifact-tag":      tag,
		},
		Body:          bodyReader,
		ContentLength: contentLength,
		ContentType:   aws.String("application/octet-stream"),
	})
	if err != nil {
		println(fmt.Sprintf("!!!!!!!!! failed put artifact %v", err))
		return fmt.Errorf("failed to store files in bucket cache: %w", err)
	}
	return nil
}

func (c *BucketClient) FetchArtifact(hash string, rawBody interface{}) (*ClientResponse, error) {
	println(fmt.Sprintf("!!!!!!!!! fetch artifact %v", hash))
	if err := c.okToRequest(); err != nil {
		return nil, err
	}
	params := url.Values{}
	c.addTeamParam(&params)
	// only add a ? if it's actually needed (makes logging cleaner)
	encoded := params.Encode()
	if encoded != "" {
		encoded = "?" + encoded
	}
	statusCode := 200
	duration := -1
	tag := ""
	var body io.ReadCloser
	resp, err := c.ServiceClient.GetObject(context.TODO(), &s3.GetObjectInput{
		Bucket:       aws.String(c.config.BucketName),
		Key:          aws.String("/v8/artifacts/" + hash + encoded),
		RequestPayer: types.RequestPayerRequester,
	})
	if err != nil {
		println(fmt.Sprintf("!!!!!!!!! fetch artifact err %v", err))
		var httpResponseError *awshttp.ResponseError
		if errors.As(err, &httpResponseError) {
			statusCode = httpResponseError.HTTPStatusCode()
		} else {
			return nil, err
		}
	}

	if resp != nil {
		if artifactDurationString, ok := resp.Metadata["artifact-duration"]; ok {
			intVar, atoiErr := strconv.Atoi(artifactDurationString)
			if atoiErr != nil {
				err = fmt.Errorf("invalid x-artifact-duration header: %w", atoiErr)
			}
			duration = intVar
		}
		if artifactTag, ok := resp.Metadata["artifact-tag"]; ok {
			tag = artifactTag
		}
		body = resp.Body
	}

	return &ClientResponse{
		StatusCode:       statusCode,
		ArtifactDuration: duration,
		Body:             body,
		Tag:              tag,
	}, err
}

func (c *BucketClient) RecordAnalyticsEvents(events []map[string]interface{}) error {
	if err := c.okToRequest(); err != nil {
		return err
	}
	params := url.Values{}
	c.addTeamParam(&params)
	encoded := params.Encode()
	if encoded != "" {
		encoded = "?" + encoded
	}
	body, err := json.Marshal(events)
	if err != nil {
		return err
	}
	bodyReader, contentLength, err := getBodyReaderAndContentLength(body)
	if err != nil {
		return fmt.Errorf("cannot store analytics: %w", err)
	}
	_, err = c.ServiceClient.PutObject(context.TODO(), &s3.PutObjectInput{
		Bucket:        aws.String(c.config.BucketName),
		Key:           aws.String(c.config.BucketPrefix + "/v8/artifacts/events" + encoded),
		RequestPayer:  types.RequestPayerRequester,
		Body:          bodyReader,
		ContentLength: contentLength,
		ContentType:   aws.String("application/json"),
	})
	if err != nil {
		return fmt.Errorf("failed to store files in bucket cache: %w", err)
	}
	return err
}

func (c *BucketClient) addTeamParam(params *url.Values) {
	if c.config.TeamId != "" && strings.HasPrefix(c.config.TeamId, "team_") {
		params.Add("teamId", c.config.TeamId)
	}
	if c.config.TeamSlug != "" {
		params.Add("slug", c.config.TeamSlug)
	}
}
