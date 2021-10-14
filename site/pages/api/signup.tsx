import { NextApiRequest, NextApiResponse } from 'next'
import { subscribeToForm } from '../../lib/ConvertKitApi'

const FORM_ID = '1939703'
export default async function handle(
  req: NextApiRequest,
  res: NextApiResponse
) {
  res.setHeader('Content-Type', 'application/json')
  if (req.method === 'POST') {
    const subscriber = await subscribeToForm(
      FORM_ID,
      req.body.email,
      req.body.firstName,
      {
        last_name: req.body.lastName,
      }
    )
    res.statusCode = 201
    res.json(subscriber)
  } else {
    res.statusCode = 404
    res.send(null)
  }
}
